//! Reading the byte upstream: how much, and from where.

use super::*;

/// How much of an upstream's output the structured half will hold.
///
/// **Nothing streams here, and that is the load-bearing fact.** Every stage materialises, so the
/// byte prefix is read to the end into a `String` before the first tool runs — and an upstream that
/// never ends therefore has no end. `yes | lines | first 2` reached **4.4 GB of resident memory in
/// three seconds** and kept going: not a hang, an OOM with a countdown, in a line that is ordinary
/// to type and that `yes | head -2` answers instantly on the byte path.
///
/// The byte path survives it because `head` *exits* and `yes` dies of `SIGPIPE`. The structured half
/// cannot do that — it has no way to say "enough" until the tools run, and the tools run after the
/// prefix has finished. Fixing that properly means running the prefix concurrently with the tool
/// half, which means splitting the fork from the wait in `run_byte_stages` and redoing the
/// `setpgid`/`tcsetpgrp` handover around it. Breaking interactive job control to fix this would be
/// a worse trade than the bug.
///
/// So the reader stops instead. At the cap the descriptor is **closed**, which is what gives the
/// upstream its `SIGPIPE` and ends it, and the pipeline **fails** — a truncated table silently
/// passed on would be a wrong answer, which is the one failure this project is built not to have.
///
/// 256 MiB, which is three orders of magnitude above anything a command prints on purpose and well
/// under what turning it into rows would then cost.
const CAPTURE_LIMIT: usize = 256 * 1024 * 1024;

/// What reading an upstream produced.
pub(super) enum Upstream {
    Read(String),
    /// Ctrl-C arrived while parked on the read.
    Interrupted,
    /// More than [`CAPTURE_LIMIT`]; the descriptor is closed and nothing may use what was read.
    TooLarge,
}

/// What to say when an upstream would not fit.
///
/// It names the cap and what to do about it, because "too much output" with no number is a message
/// a person can only guess at — and the fix is nearly always a bounded upstream, which is a thing
/// the byte path has always been good at.
pub(super) fn too_large(reader: &str) -> String {
    format!(
        "{reader}: more than {} MiB arrived before the first row. \
         The structured half holds all of its input at once, so an upstream that does not end \
         cannot be read — bound it, as in `… | head -n 1000 | lines | …`",
        CAPTURE_LIMIT / (1024 * 1024)
    )
}

/// The shell's standard input, to the end or to [`CAPTURE_LIMIT`].
///
/// Lossy rather than refusing: a tool that turns bytes into rows is being handed something the
/// user piped in, and answering "not UTF-8" for one stray byte in a log file would be worse than
/// carrying on. `Val::Bytes` exists for the cell that genuinely holds binary; this is the channel.
pub(super) fn read_standard_input() -> Upstream {
    use nix::poll::{PollFd, PollFlags, PollTimeout, poll};
    use std::io::Read;
    use std::os::fd::AsFd;

    let stdin = std::io::stdin();
    let mut handle = stdin.lock();
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        // **Waited for in slices, so Ctrl-C is heard.**
        //
        // `wc -l` reading a terminal is a child in the foreground process group, so SIGINT kills
        // it. This read happens in the shell's *own* process, where the handler only sets a flag —
        // so a blocking `read_to_end` could not be broken out of at all, and every line typed after
        // it was swallowed by the read rather than run. The flag is polled between slices, which is
        // the same thing `eval_command_list` does at every command boundary.
        match poll(
            &mut [PollFd::new(handle.as_fd(), PollFlags::POLLIN)],
            PollTimeout::from(100u16),
        ) {
            Ok(0) => {
                if crate::exec::job::interrupt_pending() {
                    return Upstream::Interrupted;
                }
                continue;
            }
            Ok(_) => {}
            // `EINTR` is the signal arriving while parked; ask the flag and carry on either way.
            Err(_) => {
                if crate::exec::job::interrupt_pending() {
                    return Upstream::Interrupted;
                }
                continue;
            }
        }
        match handle.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => {
                buffer.extend_from_slice(&chunk[..read]);
                // Closing the descriptor is what ends the upstream, so stop reading rather than
                // draining politely to an end that is not coming.
                if buffer.len() > CAPTURE_LIMIT {
                    // The buffer is dropped unread for the same reason the prefix path drops its:
                    // nothing may use a truncated stream.
                    return Upstream::TooLarge;
                }
            }
            // A signal is not the end of the stream — see `coordinates::read_bounded`.
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
    Upstream::Read(String::from_utf8_lossy(&buffer).into_owned())
}

/// Run the byte half of a mixed pipeline and collect what it printed.
///
/// The prefix runs through the ordinary path — same forks, same descriptors, same everything — with
/// stdout pointed at a pipe instead of the terminal. Nothing about how those commands execute
/// changes; only where their output goes.
pub(super) fn capture(
    env: &mut Environment,
    prefix: &Pipeline,
    fallback: fn(&mut Environment, &Pipeline) -> Result<i32>,
) -> Result<(i32, String)> {
    use std::io::Read;
    use std::os::fd::{AsRawFd, FromRawFd};

    // **`O_CLOEXEC`, so the pair does not leak into every stage of the prefix.** Without it each
    // forked command inherits a read end of the very pipe it is writing to as stdout — a descriptor
    // nothing there will ever use, and one more holder of a pipe whose lifetime decides when the
    // read below sees EOF.
    let (reader, writer) = nix::unistd::pipe2(nix::fcntl::OFlag::O_CLOEXEC)
        .map_err(|e| oslo_base::error::ShellError::ExecutionError(format!("pipe: {e}")))?;

    // stdout is put back whatever happens below, including on the error path: leaving the shell
    // writing into a closed pipe would be a far worse failure than the one being reported.
    // Through the shell's own save policy: a plain `dup` lands on the lowest free number — inside
    // the 3..9 a script addresses — and carries no `FD_CLOEXEC`, so every program the shell ran
    // inherited a copy of its stdout. See [`crate::exec::redirect::save_fd`].
    let saved = crate::exec::redirect::save_fd(std::io::stdout().as_raw_fd()).ok_or_else(|| {
        oslo_base::error::ShellError::ExecutionError("dup: cannot save stdout".to_string())
    })?;
    let _ = nix::unistd::dup2(writer.as_raw_fd(), std::io::stdout().as_raw_fd());
    drop(writer);

    // **Drained while the prefix runs, not after it.** A pipe holds 64 KiB; reading only once
    // `fallback` returned meant the prefix blocked in `write` the moment it produced more than that,
    // and `fallback` cannot return until the prefix exits. `cat big.json | from json | …` hung for
    // ever, at exactly one byte over the pipe's capacity — the documented headline example of this
    // very module, for any input a real command produces.
    let draining = std::thread::spawn(move || {
        // **`read_to_end` and then lossy, not `read_to_string`.** A prefix is an arbitrary program
        // and its output is arbitrary bytes; `read_to_string` answers `InvalidData` on the first
        // one that is not UTF-8 and leaves the buffer *empty*, so a single stray byte anywhere in a
        // two-megabyte log threw the whole of it away and `… | lines | length` said `0` with no
        // error and status 0. The head-position path four lines up already reads it this way.
        let mut buffer = Vec::new();
        let mut reader = std::fs::File::from(reader);
        // **Bounded, and the bound is enforced by closing rather than by ignoring.** Reading in
        // slices and dropping the descriptor at the cap is what sends the prefix its `SIGPIPE`; a
        // drain that kept reading and threw the excess away would leave `yes` running for ever and
        // the shell growing by a gigabyte a second. See [`CAPTURE_LIMIT`].
        let mut chunk = vec![0u8; 64 * 1024];
        let mut over = false;
        loop {
            match reader.read(&mut chunk) {
                Ok(0) => break,
                Ok(read) => {
                    buffer.extend_from_slice(&chunk[..read]);
                    if buffer.len() > CAPTURE_LIMIT {
                        over = true;
                        break;
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
        // Dropping the read end here is what ends an upstream that has no end of its own, and it
        // is what lets `fallback` below return at all.
        drop(reader);
        // Nothing may use what was read when the cap was hit, so it is not paid for: the lossy
        // conversion would double a quarter-gigabyte buffer to build a string that is thrown away.
        match over {
            true => (String::new(), true),
            false => (String::from_utf8_lossy(&buffer).into_owned(), false),
        }
    });

    let status = fallback(env, prefix);

    // **Before the join, and that order is the whole of it.** The reader sees EOF only once every
    // write end is gone: the prefix's children close theirs by exiting, which `fallback` has waited
    // for, and this puts back the shell's own — the last one.
    //
    // SAFETY: `saved` is a descriptor this function created with `dup` and has not closed.
    let saved = unsafe { std::os::fd::OwnedFd::from_raw_fd(saved) };
    let _ = nix::unistd::dup2(saved.as_raw_fd(), std::io::stdout().as_raw_fd());

    let (output, over) = draining.join().unwrap_or_default();
    if over {
        return Err(oslo_base::error::ShellError::ExecutionError(too_large(
            "the pipeline",
        )));
    }
    Ok((status?, output))
}

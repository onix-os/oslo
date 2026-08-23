//! Whether an entry may go — the three ways `rm` decides, and the prompt behind two of them.
//!
//! `-f` never asks, `-i` always asks, and in between there is the write-protected prompt: the one
//! GNU raises for a file the user cannot write to, and only when someone is at the terminal to
//! answer it. A script's stdin is not a tty, so it is silent everywhere it would otherwise hang.
//!
//! Split from the walk when the file crossed the 600-line limit. It is one idea — asking — and the
//! walk beside it is another.

use super::{Level, Walk};
use nix::fcntl::AtFlags;
use nix::unistd::{AccessFlags, faccessat};
use std::ffi::OsString;
use std::io::IsTerminal;
use std::os::fd::AsRawFd;
use std::path::Path;

/// Whether this entry may go, for something named inside an open directory.
///
/// **`-f` never asks, `-i` always asks, and in between there is the write-protected prompt** — the
/// one GNU raises for a file the user cannot write to, and only when someone is at the terminal to
/// answer it. A script's stdin is not a tty, so this is silent everywhere it would otherwise hang.
pub(super) fn ask_at(
    walk: &Walk,
    shown: &str,
    kind: &str,
    parent: &Level,
    name: &OsString,
) -> bool {
    if walk.force {
        return true;
    }
    if walk.interactive {
        return confirm(&walk.origin, &format!("remove {kind} '{shown}'"));
    }
    if !std::io::stdin().is_terminal() {
        return true;
    }
    let writable = faccessat(
        Some(parent.as_raw_fd()),
        name.as_os_str(),
        AccessFlags::W_OK,
        AtFlags::AT_EACCESS,
    )
    .is_ok();
    writable
        || confirm(
            &walk.origin,
            &format!("remove write-protected {kind} '{shown}'"),
        )
}

/// The same question about the operand, which has a path and no parent descriptor.
pub(super) fn ask_path(walk: &Walk, shown: &str, kind: &str, path: &Path) -> bool {
    if walk.force {
        return true;
    }
    if walk.interactive {
        return confirm(&walk.origin, &format!("remove {kind} '{shown}'"));
    }
    if !std::io::stdin().is_terminal() || nix::unistd::access(path, AccessFlags::W_OK).is_ok() {
        return true;
    }
    confirm(
        &walk.origin,
        &format!("remove write-protected {kind} '{shown}'"),
    )
}

/// Anything but a `y` answer means no, as it does in `rm` and in `find -ok`.
pub fn confirm(origin: &str, question: &str) -> bool {
    eprint!("{origin}rm: {question}? ");
    let _ = std::io::Write::flush(&mut std::io::stderr());
    match answer() {
        Some(answer) => matches!(answer.trim_start().chars().next(), Some('y') | Some('Y')),
        // Interrupted, or end of input: either way this entry does not go. The walk stops at its
        // next poll of `job::interrupt_waiting`, which is what turns one refused prompt into the
        // whole `rm` ending.
        None => false,
    }
}

/// One line from the terminal, or `None` if a Ctrl-C arrived while waiting for it.
///
/// **`read_line` cannot be used here, and that was the bug.** SIGINT is installed without
/// `SA_RESTART` precisely so a blocking read fails with `EINTR` rather than resuming — see
/// `exec::job::signals`. But `BufRead::read_line` is built on `read_until`, which treats
/// `ErrorKind::Interrupted` as "try again" and loops. So the keystroke set the interrupt flag, the
/// read went straight back to waiting, and nothing looked at the flag until an answer arrived that
/// was never coming:
///
/// ```text
/// oslo: rm: remove write-protected regular file '…/objects/a6/b653…'? ^C^C^C^C^C^C
/// ```
///
/// `rm` is a *builtin*, so there is no child for the terminal driver to kill — the shell itself is
/// the foreground process, and a prompt it will not abandon is a prompt nothing can escape. The
/// real `rm` dies on the first Ctrl-C because it is somebody else's process; this has to arrange
/// the same ending for itself.
///
/// So the bytes are read one at a time and `EINTR` is answered rather than retried.
///
/// # `EINTR` alone is not enough, and testing the flag first is not either
///
/// `EINTR` only reports a signal that arrived **while the process was inside `read(2)`**. One
/// delivered a moment earlier — between printing the question and entering the read — runs the
/// handler, sets the flag, and leaves nothing pending: the read then blocks on a keystroke nobody
/// is going to type. Same hang, narrower door.
///
/// So the flag is tested before every read as well. That does not *close* the window — a signal can
/// still land between the test and the `read`, which is a plain time-of-check race — but it removes
/// the wide part of it, and under a test pinned to a single core it took the failure rate from
/// about one run in fifteen to one in forty.
///
/// # Why not `poll` the descriptor and close it properly
///
/// Tried, and reverted: it hangs `rm -i`. `std::io::stdin()` is **buffered**, and this reads through
/// that buffer on purpose — it is what lets `rm -i` take its answers from the same script the shell
/// is reading, a heredoc most of all. `poll` asks the *descriptor*, which reports nothing to read
/// while the answer is already sitting in the buffer, so the loop waits for a byte that has
/// arrived. `rm_race_tests` goes from 0.12s to never finishing.
///
/// Closing it for real needs the wait and the signal unblock to be one atomic step — `ppoll` with a
/// mask, or a self-pipe the handler writes to — *and* a way to consult the buffer first. Worth
/// doing; bigger than this comment.
fn answer() -> Option<String> {
    use std::io::Read;

    let mut stdin = std::io::stdin().lock();
    let mut line = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        // Before every read, including the first: the signal may already have been and gone.
        if crate::exec::job::interrupt_waiting() {
            return None;
        }
        match stdin.read(&mut byte) {
            // End of input: `rm` treats that as "no", which is what a script piping nothing means.
            Ok(0) => break,
            Ok(_) if byte[0] == b'\n' => break,
            Ok(_) => line.push(byte[0]),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => return None,
            Err(_) => return None,
        }
    }
    Some(String::from_utf8_lossy(&line).into_owned())
}

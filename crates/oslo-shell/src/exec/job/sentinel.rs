//! Ctrl-C three times kills the foreground job outright.
//!
//! # Why this needs a second process at all
//!
//! A shell doing job control is **not in the terminal's foreground process group** — it hands that
//! to the job so the job can read the terminal and receive Ctrl-C. The kernel delivers the tty's
//! `SIGINT` to the foreground group and nowhere else, so the shell, sitting in `waitpid`, never
//! learns a key was pressed. Everything oslo knows about an interrupted command it infers *after
//! the fact*, from the child's wait status — see [`super::super::simple::external`].
//!
//! That inference is enough for a job that dies. It is no use at all for one that does not:
//!
//! ```text
//! $ sh -c 'trap "" INT; sleep 300'
//! ^C ^C ^C          nothing happens, and nothing can — the shell is not being signalled
//! ```
//!
//! and there is no keystroke that produces `SIGKILL`, because the tty driver cannot send one.
//! Something has to *observe* the Ctrl-C and then call `kill` itself, and the only way to observe
//! it is to be in the group the kernel is signalling.
//!
//! So: one small process, forked once per interactive session, that joins whichever process group
//! currently owns the terminal and counts the interrupts it receives. On the third it sends
//! `SIGKILL` to that group. It reads no input, writes no output, and holds no terminal.
//!
//! # What it will not save you from
//!
//! **A process wedged in an uninterruptible kernel call cannot be killed by anything**, `SIGKILL`
//! included — the signal is recorded and delivered when the call returns, which for a large
//! `unlink` on a slow filesystem may be a while. This helps against a program that catches or
//! ignores the signal, or spins retrying, which is the common case; it cannot help against `D`
//! state, and nothing can.
//!
//! # The counter resets per job, not on a timer
//!
//! Three presses *during one command* escalate. A window would need a clock and would make the
//! behaviour depend on how fast somebody types; "I have now asked three times and it is still
//! there" is the thing being expressed, and it is as true over ten seconds as over one.

use nix::libc;
use nix::sys::signal::{self, SaFlags, SigAction, SigHandler, SigSet, Signal};
use nix::unistd::{ForkResult, Pid, fork, setpgid};
use std::io::{Read, Write};
use std::os::unix::io::{FromRawFd, IntoRawFd};
use std::sync::Mutex;
use std::sync::atomic::{AtomicI32, AtomicU32, Ordering};

/// How many interrupts in one job before the group is killed.
const ESCALATE_AFTER: u32 = 3;

/// The pipe the shell tells the sentinel through, and nothing else.
///
/// `None` until the first foreground job, so a shell that never runs one — a script, a `-c`, a
/// session spent entirely in builtins — never pays for the fork.
static CHANNEL: Mutex<Option<std::fs::File>> = Mutex::new(None);

/// In the *sentinel*: the group it is watching, or 0 while it is watching nothing.
///
/// Read by the signal handler, so it is an atomic and nothing else.
static WATCHING: AtomicI32 = AtomicI32::new(0);

/// In the *sentinel*: interrupts seen since the current job started.
static SEEN: AtomicU32 = AtomicU32::new(0);

/// Start watching `pgid`: the group that now owns the terminal.
///
/// Called after the terminal is handed over, so the sentinel joins a group the tty is already
/// signalling. Silent about every failure — a shell that cannot fork a helper is a working shell
/// with the interrupt behaviour it had before.
pub(crate) fn watch(pgid: Pid) {
    tell(pgid.as_raw());
}

/// Stop watching: the job is over and the shell has the terminal back.
pub(crate) fn stand_down() {
    tell(0);
}

/// Send a group id to the sentinel, starting it if this is the first foreground job.
fn tell(pgid: i32) {
    let Ok(mut channel) = CHANNEL.lock() else {
        return;
    };
    if channel.is_none() {
        // Nothing to stand down from if it was never started, and starting one to immediately tell
        // it to do nothing would fork on the way *out* of the first job.
        if pgid == 0 {
            return;
        }
        *channel = start();
    }
    if let Some(pipe) = channel.as_mut()
        && pipe.write_all(&pgid.to_ne_bytes()).is_err()
    {
        // The sentinel is gone. Forget it rather than retrying every command; the shell is
        // otherwise unaffected.
        *channel = None;
    }
}

/// Fork the sentinel, answering the pipe to talk to it with.
fn start() -> Option<std::fs::File> {
    let (read, write) = nix::unistd::pipe().ok()?;
    // SAFETY: the child calls only syscalls — no allocation, no locks, no Rust destructors that
    // could be waiting on a mutex another thread held at the moment of the fork.
    match unsafe { fork() } {
        Ok(ForkResult::Child) => {
            drop(write);
            run(read.into_raw_fd());
        }
        Ok(ForkResult::Parent { .. }) => {
            drop(read);
            Some(unsafe { std::fs::File::from_raw_fd(write.into_raw_fd()) })
        }
        Err(_) => None,
    }
}

/// The sentinel itself. Never returns.
fn run(orders: std::os::unix::io::RawFd) -> ! {
    // Die with the shell. Without this a sentinel outlives a shell that was killed rather than
    // exited, and would sit in a process group forever.
    let _ = nix::sys::prctl::set_pdeathsig(Signal::SIGKILL);

    // Count interrupts rather than dying of them, and stay out of the way of everything else the
    // terminal can send: the sentinel is in the job's process group, so it receives the job's
    // Ctrl-Z and Ctrl-\ as well, and it must not stop or dump core in response.
    let counting = SigAction::new(
        SigHandler::Handler(on_interrupt),
        SaFlags::empty(),
        SigSet::empty(),
    );
    let ignored = SigAction::new(SigHandler::SigIgn, SaFlags::empty(), SigSet::empty());
    unsafe {
        let _ = signal::sigaction(Signal::SIGINT, &counting);
        for sig in [
            Signal::SIGTSTP,
            Signal::SIGTTIN,
            Signal::SIGTTOU,
            Signal::SIGQUIT,
            Signal::SIGHUP,
        ] {
            let _ = signal::sigaction(sig, &ignored);
        }
    }

    let mut orders = unsafe { std::fs::File::from_raw_fd(orders) };
    let mut word = [0u8; 4];
    loop {
        match orders.read_exact(&mut word) {
            Ok(()) => {}
            // The shell closed the pipe, which means the shell is gone.
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            // A signal arrived mid-read. That is the ordinary case here.
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
        let pgid = i32::from_ne_bytes(word);
        // Set before joining, so an interrupt delivered during the `setpgid` is attributed to the
        // job rather than to whatever the sentinel was in before.
        SEEN.store(0, Ordering::SeqCst);
        WATCHING.store(pgid, Ordering::SeqCst);
        if pgid != 0 {
            // `ESRCH` when the job finished before this ran, which is a race the shell wins and
            // the sentinel does not need to care about: the next order will be a 0.
            let _ = setpgid(Pid::from_raw(0), Pid::from_raw(pgid));
        }
    }
    std::process::exit(0);
}

/// The sentinel's `SIGINT` handler: count, and kill the group on the third.
///
/// Touches two atomics and calls `kill`, all of which are async-signal-safe. Nothing here
/// allocates or takes a lock.
extern "C" fn on_interrupt(_: libc::c_int) {
    let pgid = WATCHING.load(Ordering::SeqCst);
    if pgid == 0 {
        return;
    }
    if SEEN.fetch_add(1, Ordering::SeqCst) + 1 < ESCALATE_AFTER {
        return;
    }
    // **Leave the group before killing it.** The sentinel is *in* the group it is about to signal,
    // and `kill(-pgid, SIGKILL)` does not spare the sender — it would take itself with the job,
    // the pipe would close, and the shell would have to fork a replacement for the next command.
    // `setpgid` is on the list of calls a handler may make, as are `kill` and the atomics here.
    unsafe {
        libc::setpgid(0, 0);
        // The whole group, because a job is its process group — killing the leader alone would
        // leave a pipeline's other stages and any grandchildren behind.
        libc::kill(-pgid, libc::SIGKILL);
    }
    // And stop watching: the group is gone, and its id could in principle be reused by a later
    // job that has not asked to be killed.
    WATCHING.store(0, Ordering::SeqCst);
}

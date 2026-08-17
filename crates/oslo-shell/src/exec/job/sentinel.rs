//! Ctrl-C enough times takes the terminal back from a job that will not listen.
//!
//! Off unless `oslo.misc.interrupt_escape` is set to how many presses it should take. Nothing here
//! runs — not even the fork — in a shell that has not asked for it.
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
//! Something has to *observe* the Ctrl-C and act, and the only way to observe it is to be in the
//! group the kernel is signalling.
//!
//! So: one small process, forked once per interactive session, that joins whichever process group
//! currently owns the terminal and counts the interrupts it receives. It reads no input, writes no
//! output, and holds no terminal.
//!
//! # It stops the job; it does not kill it
//!
//! `SIGSTOP`, not `SIGKILL`, and the difference is most of the value. It cannot be caught or
//! ignored — so it works on exactly the programs this exists for — and `waitpid` already returns
//! `Stopped` for it, which means the shell's existing path takes over: the job goes into the job
//! table, the terminal comes back, and the prompt returns. Nothing is destroyed. `fg`, `bg` and
//! `kill %1` all then work on it, so the decision about what to do with a job that will not
//! listen stays with the person, where it belongs.
//!
//! That also means there is **no new signal aimed at the shell**, and so nothing for a user `trap`
//! to collide with — a shell-side handler could be replaced by `trap ... USR1` and the feature
//! would silently stop working.
//!
//! # What it will not save you from
//!
//! **A process wedged in an uninterruptible kernel call.** `SIGSTOP` is recorded and delivered when
//! the syscall returns, exactly as `SIGKILL` is, so a task blocked on a dead NFS mount is beyond
//! this and beyond everything else. What this helps with is a program that catches or ignores the
//! interrupt, or spins retrying it, which is the case a person can otherwise do nothing about.
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

/// In the *sentinel*: how many interrupts in one job before it acts. Sent by the shell, so the
/// setting lives in one place and the sentinel needs no configuration of its own.
static ESCALATE_AFTER: AtomicU32 = AtomicU32::new(0);

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
pub(crate) fn watch(pgid: Pid, after: u32) {
    tell(pgid.as_raw(), after);
}

/// Stop watching: the job is over and the shell has the terminal back.
pub(crate) fn stand_down() {
    tell(0, 0);
}

/// Send a group id to the sentinel, starting it if this is the first foreground job.
fn tell(pgid: i32, after: u32) {
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
        && pipe
            .write_all(&[pgid.to_ne_bytes(), after.to_ne_bytes()].concat())
            .is_err()
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
    let mut word = [0u8; 8];
    loop {
        match orders.read_exact(&mut word) {
            Ok(()) => {}
            // The shell closed the pipe, which means the shell is gone.
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            // A signal arrived mid-read. That is the ordinary case here.
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
        let pgid = i32::from_ne_bytes([word[0], word[1], word[2], word[3]]);
        let after = u32::from_ne_bytes([word[4], word[5], word[6], word[7]]);
        // Set before joining, so an interrupt delivered during the `setpgid` is attributed to the
        // job rather than to whatever the sentinel was in before.
        SEEN.store(0, Ordering::SeqCst);
        ESCALATE_AFTER.store(after, Ordering::SeqCst);
        WATCHING.store(pgid, Ordering::SeqCst);
        if pgid != 0 {
            // `ESRCH` when the job finished before this ran, which is a race the shell wins and
            // the sentinel does not need to care about: the next order will be a 0.
            let _ = setpgid(Pid::from_raw(0), Pid::from_raw(pgid));
        }
    }
    std::process::exit(0);
}

/// The sentinel's `SIGINT` handler: count, and stop the group once there have been enough.
///
/// Touches three atomics and calls `setpgid` and `kill`, all of which POSIX lists as safe to call
/// from a handler. Nothing here allocates or takes a lock.
extern "C" fn on_interrupt(_: libc::c_int) {
    let pgid = WATCHING.load(Ordering::SeqCst);
    let after = ESCALATE_AFTER.load(Ordering::SeqCst);
    if pgid == 0 || after == 0 {
        return;
    }
    if SEEN.fetch_add(1, Ordering::SeqCst) + 1 < after {
        return;
    }
    // **Leave the group before signalling it.** The sentinel is *in* the group it is about to
    // signal, and `kill(-pgid, …)` does not spare the sender — a `SIGSTOP` would stop the sentinel
    // along with the job, and it would never read another order.
    unsafe {
        libc::setpgid(0, 0);
        // **`SIGSTOP`, and the whole group.** It cannot be caught or ignored, which is the entire
        // point — the programs this exists for are the ones that caught the interrupt. `waitpid`
        // then returns `Stopped`, and the shell's own path for Ctrl-Z takes over from there: the
        // job is recorded, the terminal comes back, the prompt returns. Nothing is destroyed, and
        // `fg`, `bg` and `kill %1` all work on what is left.
        libc::kill(-pgid, libc::SIGSTOP);
    }
    // And stop watching: the shell is about to send a fresh order for the next job, and until it
    // does there is nothing here worth acting on.
    WATCHING.store(0, Ordering::SeqCst);
}

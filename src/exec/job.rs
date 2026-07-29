//! Job-control state and the signal dispositions that go with it.
//!
//! Two halves that have to agree: [`JobManager::setup_signals`] decides what the *shell* ignores
//! so a keystroke aimed at a foreground job cannot kill the REPL, and
//! [`reset_signals_for_child`] undoes exactly that in every process the shell forks.

use nix::libc;
use nix::sys::signal::{self, SigAction, SigHandler, SigSet, SigmaskHow, Signal};
use nix::unistd::Pid;
use std::sync::atomic::{AtomicBool, Ordering};

/// Every signal whose disposition the shell may have changed for its own benefit.
///
/// SIGPIPE is in the list even though the shell never touches it deliberately: the Rust runtime
/// sets it to `SIG_IGN` before `main` so a write to a closed socket returns `EPIPE` instead of
/// killing the process. An ignored disposition survives `execv` (only *handled* signals are reset
/// by exec), so without this every command rush runs inherits it — which is why `yes | head -1`
/// printed `yes: standard output: Broken pipe` instead of dying quietly on the closed pipe.
const RESET_IN_CHILD: [Signal; 6] = [
    Signal::SIGPIPE,
    Signal::SIGINT,
    Signal::SIGQUIT,
    Signal::SIGTSTP,
    Signal::SIGTTIN,
    Signal::SIGTTOU,
];

/// Restore the signal state a freshly-started program is entitled to assume.
///
/// Call this in the child between `fork` and `execv`, and in any forked subshell before it starts
/// running commands. It is the counterpart of [`JobManager::setup_signals`]: the REPL ignores
/// SIGTSTP/SIGTTIN/SIGTTOU so that job-control keystrokes and terminal access from a background
/// process cannot stop the shell itself, but a child that inherits those cannot be suspended at
/// all — Ctrl-Z on anything rush launched did nothing.
///
/// Only `sigaction` and `sigprocmask` are used, both async-signal-safe, so this is legal in the
/// window after `fork` where almost nothing else is.
pub fn reset_signals_for_child() {
    let dfl = SigAction::new(
        SigHandler::SigDfl,
        signal::SaFlags::empty(),
        SigSet::empty(),
    );
    for sig in RESET_IN_CHILD {
        // Errors are unreportable here (the child has not exec'd yet and stderr may belong to a
        // pipe the parent is about to close); a failure leaves the inherited disposition, which
        // is no worse than not trying.
        unsafe {
            let _ = signal::sigaction(sig, &dfl);
        }
    }

    // A blocked signal also survives exec. Nothing in rush blocks signals today, but a mask
    // inherited from whatever started the shell would be passed on to every command it runs.
    let _ = signal::sigprocmask(SigmaskHow::SIG_SETMASK, Some(&SigSet::empty()), None);
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobState {
    Running,
    Stopped,
    Completed(i32),
}

#[derive(Debug, Clone)]
pub struct Job {
    pub id: usize,
    pub pgid: Pid,
    pub pids: Vec<Pid>,
    pub command: String,
    pub state: JobState,
}

pub struct JobManager {
    pub jobs: Vec<Job>,
    pub next_id: usize,
    pub is_interactive: bool,
}

static SIGINT_RECEIVED: AtomicBool = AtomicBool::new(false);

extern "C" fn handle_sigint(_: libc::c_int) {
    SIGINT_RECEIVED.store(true, Ordering::SeqCst);
}

impl Default for JobManager {
    fn default() -> Self {
        Self::new()
    }
}

impl JobManager {
    pub fn new() -> Self {
        Self {
            jobs: Vec::new(),
            next_id: 1,
            is_interactive: false,
        }
    }

    pub fn setup_signals(&mut self) {
        let handler = SigHandler::Handler(handle_sigint);
        let action = SigAction::new(
            handler,
            signal::SaFlags::SA_RESTART,
            signal::SigSet::empty(),
        );
        unsafe {
            let _ = signal::sigaction(Signal::SIGINT, &action);
            let _ = signal::sigaction(
                Signal::SIGTSTP,
                &SigAction::new(
                    SigHandler::SigIgn,
                    signal::SaFlags::empty(),
                    signal::SigSet::empty(),
                ),
            );
            let _ = signal::sigaction(
                Signal::SIGTTIN,
                &SigAction::new(
                    SigHandler::SigIgn,
                    signal::SaFlags::empty(),
                    signal::SigSet::empty(),
                ),
            );
            let _ = signal::sigaction(
                Signal::SIGTTOU,
                &SigAction::new(
                    SigHandler::SigIgn,
                    signal::SaFlags::empty(),
                    signal::SigSet::empty(),
                ),
            );
        }
    }

    pub fn check_sigint(&self) -> bool {
        SIGINT_RECEIVED.swap(false, Ordering::SeqCst)
    }

    pub fn add_job(&mut self, pgid: Pid, pids: Vec<Pid>, command: String) -> usize {
        let id = self.next_id;
        self.next_id += 1;
        self.jobs.push(Job {
            id,
            pgid,
            pids,
            command,
            state: JobState::Running,
        });
        id
    }
}

#[cfg(test)]
mod tests {
    use super::{RESET_IN_CHILD, reset_signals_for_child};
    use nix::libc;
    use nix::sys::wait::{WaitStatus, waitpid};
    use nix::unistd::{ForkResult, fork};

    /// Exercised in a forked child, not in the test process.
    ///
    /// Setting SIGPIPE back to `SIG_DFL` here would arm the whole test binary to be killed by any
    /// write to a closed pipe, and libtest runs these on shared threads. The child never
    /// allocates — only `sigaction`, `sigprocmask` and `_exit`, all async-signal-safe — so it is
    /// safe in the post-fork window even though the parent is multi-threaded.
    #[test]
    fn every_ignored_signal_comes_back_as_sig_dfl() {
        let child = unsafe { fork() }.expect("fork");
        match child {
            ForkResult::Child => {
                let status = unsafe { child_checks_dispositions() };
                unsafe { libc::_exit(status) };
            }
            ForkResult::Parent { child } => {
                assert_eq!(
                    waitpid(child, None).expect("waitpid"),
                    WaitStatus::Exited(child, 0),
                    "a signal was left ignored, or the mask was left blocked"
                );
            }
        }
    }

    /// Ignore and block everything the helper is supposed to undo, undo it, then read the state
    /// back out of the kernel. Exit 0 means the helper did its job.
    ///
    /// # Safety
    ///
    /// Only for use in a forked child: it changes process-wide signal state.
    unsafe fn child_checks_dispositions() -> i32 {
        unsafe {
            let mut ign: libc::sigaction = std::mem::zeroed();
            ign.sa_sigaction = libc::SIG_IGN;
            let mut full: libc::sigset_t = std::mem::zeroed();
            libc::sigfillset(&mut full);

            for sig in RESET_IN_CHILD {
                if libc::sigaction(sig as i32, &ign, std::ptr::null_mut()) != 0 {
                    return 10;
                }
            }
            if libc::sigprocmask(libc::SIG_SETMASK, &full, std::ptr::null_mut()) != 0 {
                return 11;
            }

            reset_signals_for_child();

            for sig in RESET_IN_CHILD {
                let mut cur: libc::sigaction = std::mem::zeroed();
                if libc::sigaction(sig as i32, std::ptr::null(), &mut cur) != 0 {
                    return 12;
                }
                if cur.sa_sigaction != libc::SIG_DFL {
                    return 13;
                }
            }

            let mut mask: libc::sigset_t = std::mem::zeroed();
            if libc::sigprocmask(libc::SIG_SETMASK, std::ptr::null(), &mut mask) != 0 {
                return 14;
            }
            for sig in RESET_IN_CHILD {
                if libc::sigismember(&mask, sig as i32) != 0 {
                    return 15;
                }
            }
            0
        }
    }
}

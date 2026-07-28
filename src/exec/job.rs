use nix::libc;
use nix::sys::signal::{self, SigAction, SigHandler, Signal};
use nix::unistd::Pid;
use std::sync::atomic::{AtomicBool, Ordering};

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

//! The job table: what the shell remembers about the children it is not currently waiting for.
//!
//! R7.2/R7.4. Before this existed, `JobManager::add_job` had no callers, [`JobState::Stopped`] was
//! unreachable, and a background child that finished stayed a zombie for the shell's lifetime
//! because nothing ever waited for it. Both halves are fixed here: every background job and every
//! stopped foreground job is recorded, and [`reap_background_jobs`] — called at every command
//! boundary by [`crate::exec::pipeline::eval_command_list`] — collects the ones that have ended.
//!
//! # The reaping invariant
//!
//! [`reap_background_jobs`] runs at command boundaries and asks after each *known* pid with
//! `WNOHANG`, never `waitpid(-1)`. Both would collect the same zombies, but `-1` also collects
//! children this table has never heard of — and the statuses it collects are kept in
//! [`JobTable::take_status`] so that a later `wait` can still report them, which is what keeps
//! opportunistic reaping from stealing `wait $!`'s answer.

use nix::sys::wait::WaitStatus;
use nix::unistd::Pid;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};

/// Where a job is in its life.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobState {
    Running,
    Stopped,
    Completed(i32),
}

/// One entry of the job table: a process group the shell started and can still name.
#[derive(Debug, Clone)]
pub struct Job {
    /// The `%n` a user types. Reused once the table empties, as bash's are.
    pub id: usize,
    pub pgid: Pid,
    /// The processes still expected to report. Empty once the job has ended.
    pub pids: Vec<Pid>,
    /// The processes that already have, kept so the job stays findable by any of its pids.
    ///
    /// `wait $!` names a *pid*, and `$!` is a pipeline's last stage rather than its leader, so a
    /// job cannot be looked up by pgid instead. Dropping the pid outright left the finished entry
    /// unreachable and so never removed, and a later bare `wait -n` handed the same child back.
    pub ended: Vec<Pid>,
    /// The text the user typed, for `jobs` and for the `[1]+ Done` notice.
    pub command: String,
    pub state: JobState,
    /// Whether the user has already been told about the state this entry is in.
    ///
    /// A finished job is not dropped the instant it is reaped: bash reports it once — `jobs`
    /// shows a `Done` line, or the shell prints one — and only then forgets it.
    pub notified: bool,
    /// Whether the job is running detached from the terminal, which decides the trailing `&`.
    ///
    /// Not derivable from `state`: a job stopped with Ctrl-Z and one backgrounded with `&` are
    /// both `Running` after `bg`, but only a job the shell never had in the foreground prints
    /// `sleep 1 &`. `bg` sets this and `fg` clears it, which is exactly when bash's line changes.
    pub background: bool,
}

/// Every job the shell can still name, plus the statuses of the ones it has already reaped.
#[derive(Debug, Default)]
pub struct JobTable {
    jobs: Vec<Job>,
    /// `%%`/`%+`: the job `fg` and `bg` act on by default.
    current: Option<usize>,
    /// `%-`: the one before that.
    previous: Option<usize>,
    /// Reaped exit statuses, newest last, so `wait PID` still has an answer for a child the
    /// opportunistic reaper collected first.
    statuses: Vec<(Pid, i32)>,
    /// Children that left the job table while still alive, and must be reaped anyway.
    ///
    /// **`disown` gives up managing a job, not being its parent.** The kernel does not consult the
    /// job table: a child of this process stays a zombie until this process waits for it, and
    /// nothing else can. Dropping the pid with the job left a `<defunct>` entry for the rest of the
    /// session — reproduced, not theorised.
    ///
    /// Nothing is remembered about them but the pid. They have no job, no status anybody can ask
    /// for, and no report: the only obligation left is the one the kernel imposes.
    orphans: Vec<Pid>,
    /// What has happened to jobs and their processes since anybody last asked.
    ///
    /// **Recorded here and fired elsewhere, deliberately.** Everything that mutates this table does
    /// so holding its mutex, and a Lua handler is entitled to ask the shell about its jobs — so
    /// calling one from inside would be a handler waiting for a lock its own caller holds. The
    /// transitions queue instead, and [`super::reap`] drains and fires them once the lock is gone.
    transitions: Vec<Transition>,
}

/// Something that happened to a job or one of its processes, waiting to be announced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Transition {
    /// One process of a job ended.
    ProcessExit {
        pid: Pid,
        job: Option<usize>,
        status: i32,
    },
    /// A job moved between running, stopped and ended.
    JobState {
        id: usize,
        pgid: Pid,
        text: String,
        from: &'static str,
        to: &'static str,
        background: bool,
    },
}

impl JobState {
    /// The word a hook is given for this state.
    fn word(&self) -> &'static str {
        match self {
            JobState::Running => "running",
            JobState::Stopped => "stopped",
            JobState::Completed(_) => "ended",
        }
    }
}

/// How many statuses to remember for a `wait` that may never come.
///
/// Unbounded growth is a real leak in a long-lived interactive shell that backgrounds jobs and
/// never waits for them; a fan-out of more than this many un-waited children is not a pattern a
/// shell script has.
const REMEMBERED_STATUSES: usize = 64;

/// How many finished jobs a *non-interactive* shell keeps for a `jobs` that may never run.
const REMEMBERED_COMPLETIONS: usize = 64;

/// Children the shell believes are still alive, as a lock-free fast path.
///
/// [`reap_background_jobs`] runs at every command boundary, including every iteration of a
/// shell-level loop. Taking a mutex and entering the kernel there would tax the common case —
/// no background jobs at all — for nothing.
pub(super) static LIVE_CHILDREN: AtomicUsize = AtomicUsize::new(0);

fn table() -> &'static Mutex<JobTable> {
    static TABLE: OnceLock<Mutex<JobTable>> = OnceLock::new();
    TABLE.get_or_init(|| Mutex::new(JobTable::default()))
}

/// Borrow the process-wide job table.
///
/// The entry point for everything outside this module — `jobs`, `fg`, `bg`, `disown`, `wait`.
/// A poisoned lock is recovered rather than propagated: a panic in one evaluation must not make
/// the shell unable to see its own jobs.
pub fn with_jobs<R>(f: impl FnOnce(&mut JobTable) -> R) -> R {
    let mut guard: MutexGuard<'_, JobTable> = match table().lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    f(&mut guard)
}

impl JobTable {
    /// Every job, in the order they were started.
    pub fn jobs(&self) -> &[Job] {
        &self.jobs
    }

    pub fn get(&self, id: usize) -> Option<&Job> {
        self.jobs.iter().find(|j| j.id == id)
    }

    pub fn get_mut(&mut self, id: usize) -> Option<&mut Job> {
        self.jobs.iter_mut().find(|j| j.id == id)
    }

    /// The `+` job: what `fg`/`bg`/`%%` mean with no argument.
    pub fn current_id(&self) -> Option<usize> {
        self.current
    }

    /// The `-` job: what `%-` means.
    pub fn previous_id(&self) -> Option<usize> {
        self.previous
    }

    /// The marker `jobs` prints after the job number: `+`, `-`, or a space.
    pub fn marker(&self, id: usize) -> char {
        match (self.current, self.previous) {
            (Some(cur), _) if cur == id => '+',
            (_, Some(prev)) if prev == id => '-',
            _ => ' ',
        }
    }

    /// Record a job the shell started in the background.
    pub fn add_background(&mut self, pgid: Pid, pids: Vec<Pid>, command: String) -> usize {
        self.add(pgid, pids, command, JobState::Running, true)
    }

    /// Record a foreground job that a stop signal took away from the shell.
    ///
    /// R7.2: this is the only way [`JobState::Stopped`] is ever reached. Without it, Ctrl-Z left
    /// a process suspended with nothing in the shell able to name it, so `fg` could not exist.
    pub fn add_stopped(&mut self, pgid: Pid, pids: Vec<Pid>, command: String) -> usize {
        self.add(pgid, pids, command, JobState::Stopped, false)
    }

    fn add(
        &mut self,
        pgid: Pid,
        pids: Vec<Pid>,
        command: String,
        state: JobState,
        background: bool,
    ) -> usize {
        // Job numbers restart at 1 once the table empties, as bash's do: `%1` should mean "the
        // one job I have", not "the 4001st thing this session ever backgrounded".
        let id = self.jobs.iter().map(|j| j.id).max().unwrap_or(0) + 1;
        LIVE_CHILDREN.fetch_add(pids.len(), Ordering::SeqCst);
        self.jobs.push(Job {
            id,
            pgid,
            pids,
            ended: Vec::new(),
            command,
            state,
            notified: false,
            background,
        });
        self.promote(id);
        id
    }

    /// Make `id` the current job, demoting the old current one to `%-`.
    pub fn promote(&mut self, id: usize) {
        if self.current == Some(id) {
            return;
        }
        self.previous = self.current;
        self.current = Some(id);
    }

    /// Forget a job without waiting for it — `disown`, or a job the user has been told about.
    pub fn remove(&mut self, id: usize) -> Option<Job> {
        let index = self.jobs.iter().position(|j| j.id == id)?;
        let job = self.jobs.remove(index);
        if self.current == Some(id) {
            self.current = self.previous.take();
        } else if self.previous == Some(id) {
            self.previous = None;
        }
        // **Still ours to reap, even when it is no longer ours to manage.** A completed job's pids
        // were reaped as they ended and are already accounted for; anything still live here is a
        // child of this process that only this process can collect, so it moves to [`orphans`]
        // rather than being forgotten. `LIVE_CHILDREN` keeps counting it for the same reason: it is
        // the fast path's "is there anything to reap", and the answer is still yes.
        if !matches!(job.state, JobState::Completed(_)) {
            for pid in &job.pids {
                if !self.orphans.contains(pid) {
                    self.orphans.push(*pid);
                }
            }
        }
        Some(job)
    }

    /// Resolve a job spec — `%%`, `%+`, `%-`, `%n`, `%prefix`, `%?substring` — or a bare number.
    ///
    /// Returns the job id, not an index: ids are what the user sees and what survives a removal.
    pub fn lookup(&self, spec: &str) -> Option<usize> {
        let body = spec.strip_prefix('%').unwrap_or(spec);
        match body {
            "" | "%" | "+" => self.current,
            "-" => self.previous,
            _ => {
                if let Ok(n) = body.parse::<usize>() {
                    return self.get(n).map(|j| j.id);
                }
                if let Some(needle) = body.strip_prefix('?') {
                    return self
                        .jobs
                        .iter()
                        .find(|j| j.command.contains(needle))
                        .map(|j| j.id);
                }
                self.jobs
                    .iter()
                    .find(|j| j.command.starts_with(body))
                    .map(|j| j.id)
            }
        }
    }

    /// The job that owns `pid`, if the shell still remembers one.
    ///
    /// Processes that have already ended count: a job is not un-named by finishing, and the `wait`
    /// that reports it has to reach the same entry in order to retire it.
    pub fn job_of_pid(&self, pid: Pid) -> Option<usize> {
        self.jobs
            .iter()
            .find(|j| j.pids.contains(&pid) || j.ended.contains(&pid))
            .map(|j| j.id)
    }

    /// Take the exit status the reaper collected for `pid`, if it collected one.
    ///
    /// R7.5 depends on this: once `waitpid(-1, WNOHANG)` has reaped a background child there is
    /// nothing left for `wait $!` to wait for, and the kernel answers `ECHILD`. The status has to
    /// come from here instead, and it is removed as it is read so that a second `wait` on the same
    /// pid reports "no such job" rather than the same answer twice.
    ///
    /// Forgetting on read is the *POSIX* rule, and it is a real fork in behaviour: `bash --posix`
    /// answers the second `wait $p` with 127 while default bash answers with the status again.
    /// oslo follows the POSIX side, which is also the oracle the differential corpus runs against.
    pub fn take_status(&mut self, pid: Pid) -> Option<i32> {
        let index = self.statuses.iter().position(|(p, _)| *p == pid)?;
        Some(self.statuses.remove(index).1)
    }

    /// Take everything that has happened since the last drain, for a caller that no longer holds
    /// the lock and can therefore afford to run Lua.
    pub fn take_transitions(&mut self) -> Vec<Transition> {
        std::mem::take(&mut self.transitions)
    }

    /// Note a job's move from one state to another, if it is a move at all.
    fn note_state(&mut self, id: usize, from: &JobState, to: &JobState) {
        if from.word() == to.word() {
            return;
        }
        let (from, to) = (from.word(), to.word());
        if let Some(job) = self.get(id) {
            self.transitions.push(Transition::JobState {
                id,
                pgid: job.pgid,
                text: job.command.clone(),
                from,
                to,
                background: job.background,
            });
        }
    }

    /// Record a status somebody else's `waitpid` collected, so a later `wait` still has an answer.
    ///
    /// **For a child reaped on the way to another one.** `wait -n` waits with `waitpid(-1)` and
    /// gets whichever ends first; the kernel will not offer that one again, so unless the status is
    /// kept here the child's exit code is gone for good. The job accounting is the same as the
    /// reaper's — the process leaves its job, and the job ends when its last one does.
    pub fn keep_status(&mut self, pid: Pid, code: i32) {
        let job_id = self.job_of_pid(pid);
        self.finish(job_id, pid, code);
    }

    /// Every pid the shell still expects to outlive the current command.
    pub fn live_pids(&self) -> Vec<Pid> {
        self.jobs
            .iter()
            .filter(|j| j.state != JobState::Stopped)
            .filter(|j| !matches!(j.state, JobState::Completed(_)))
            .flat_map(|j| j.pids.iter().copied())
            .collect()
    }

    /// Drop the oldest finished jobs a script never asked about.
    ///
    /// A non-interactive shell has no prompt to report at, so a `Done` entry waits for a `jobs`
    /// or a `wait` that may never come. bash bounds its list the same way rather than growing it
    /// for the life of the shell; the cap is high enough that a script which *does* report its
    /// jobs always finds them.
    pub(super) fn forget_stale_completions(&mut self) {
        let done: Vec<usize> = self
            .jobs
            .iter()
            .filter(|job| matches!(job.state, JobState::Completed(_)))
            .map(|job| job.id)
            .collect();
        for id in done
            .iter()
            .take(done.len().saturating_sub(REMEMBERED_COMPLETIONS))
        {
            self.remove(*id);
        }
    }

    /// Every pid the reaper should ask about: a stopped job's processes are still alive, and a
    /// stopped job is exactly the one that may be resumed and finish while nobody is watching.
    pub(super) fn reapable_pids(&self) -> Vec<Pid> {
        self.jobs
            .iter()
            .filter(|j| !matches!(j.state, JobState::Completed(_)))
            .flat_map(|j| j.pids.iter().copied())
            // The disowned, who have no job to be found under and are still this process's to bury.
            .chain(self.orphans.iter().copied())
            .collect()
    }

    fn remember_status(&mut self, pid: Pid, status: i32) {
        self.statuses.retain(|(p, _)| *p != pid);
        self.statuses.push((pid, status));
        if self.statuses.len() > REMEMBERED_STATUSES {
            self.statuses.remove(0);
        }
    }

    /// Fold one `waitpid` result into the table.
    pub(super) fn record(&mut self, status: WaitStatus) {
        let Some(pid) = status.pid() else { return };
        let job_id = self.job_of_pid(pid);
        match status {
            WaitStatus::Exited(_, code) => self.finish(job_id, pid, code),
            WaitStatus::Signaled(_, sig, _) => self.finish(job_id, pid, 128 + sig as i32),
            WaitStatus::Stopped(_, _) => {
                if let Some(id) = job_id {
                    self.promote(id);
                    let was = self.get(id).map(|job| job.state.clone());
                    if let Some(job) = self.get_mut(id) {
                        job.state = JobState::Stopped;
                        job.notified = false;
                    }
                    if let Some(was) = was {
                        self.note_state(id, &was, &JobState::Stopped);
                    }
                }
            }
            WaitStatus::Continued(_) => {
                if let Some(id) = job_id {
                    let was = self.get(id).map(|job| job.state.clone());
                    if let Some(job) = self.get_mut(id) {
                        job.state = JobState::Running;
                    }
                    if let Some(was) = was {
                        self.note_state(id, &was, &JobState::Running);
                    }
                }
            }
            _ => {}
        }
    }

    /// Drop `pid` from the disowned, answering whether it was one of them.
    fn forget_orphan(&mut self, pid: Pid) -> bool {
        match self.orphans.iter().position(|p| *p == pid) {
            Some(at) => {
                self.orphans.remove(at);
                true
            }
            None => false,
        }
    }

    /// Stop expecting anything from `pid`: it is gone and left no status behind.
    pub(super) fn abandon(&mut self, pid: Pid) {
        forget_children(1);
        if self.forget_orphan(pid) {
            return;
        }
        let Some(job) = self.job_of_pid(pid).and_then(|id| self.get_mut(id)) else {
            return;
        };
        job.pids.retain(|p| *p != pid);
        job.ended.push(pid);
        if job.pids.is_empty() {
            job.state = JobState::Completed(0);
            job.notified = false;
        }
    }

    /// A child of this job has ended; the job itself ends when its last process does.
    fn finish(&mut self, job_id: Option<usize>, pid: Pid, code: i32) {
        forget_children(1);
        // **A disowned child is buried and not remembered.** There is nobody who may ask: `wait`
        // on a disowned pid is "not a child of this shell" in every shell, so keeping its status
        // would only crowd out one somebody can still use.
        if self.forget_orphan(pid) {
            return;
        }
        self.remember_status(pid, code);
        self.transitions.push(Transition::ProcessExit {
            pid,
            job: job_id,
            status: code,
        });
        let Some(job) = job_id.and_then(|id| self.get_mut(id)) else {
            return;
        };
        job.pids.retain(|p| *p != pid);
        job.ended.push(pid);
        if job.pids.is_empty() {
            // A pipeline's status is its last stage's, and the last stage is the last pid left.
            let was = job.state.clone();
            job.state = JobState::Completed(code);
            job.notified = false;
            if let Some(id) = job_id {
                self.note_state(id, &was, &JobState::Completed(code));
            }
        }
    }
}

/// Drop `n` from the live-child count without ever wrapping below zero.
///
/// The count is a hint for the fast path, not a fact: a child the shell forked but never recorded
/// — a command substitution whose reader raced the reaper — would otherwise take it negative and
/// disable reaping for the rest of the session.
fn forget_children(n: usize) {
    let _ = LIVE_CHILDREN.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |live| {
        Some(live.saturating_sub(n))
    });
}

#[cfg(test)]
mod tests {
    use super::JobTable;
    use nix::unistd::Pid;

    fn table_with(names: &[&str]) -> JobTable {
        let mut table = JobTable::default();
        for (i, name) in names.iter().enumerate() {
            table.add_background(
                Pid::from_raw(1000 + i as i32),
                vec![Pid::from_raw(1000 + i as i32)],
                (*name).to_string(),
            );
        }
        table
    }

    /// `%%`, `%+` and `%-` are the specs a user types most; they follow the *order jobs were
    /// started*, with the newest current.
    #[test]
    fn current_and_previous_track_the_two_newest_jobs() {
        let table = table_with(&["sleep 1", "sleep 2", "sleep 3"]);
        assert_eq!(table.current_id(), Some(3));
        assert_eq!(table.previous_id(), Some(2));
        assert_eq!(table.lookup("%%"), Some(3));
        assert_eq!(table.lookup("%+"), Some(3));
        assert_eq!(table.lookup("%-"), Some(2));
        assert_eq!(table.marker(3), '+');
        assert_eq!(table.marker(2), '-');
        assert_eq!(table.marker(1), ' ');
    }

    /// The other spec forms: a number, a command prefix, and a `?substring`.
    #[test]
    fn job_specs_name_a_job_by_number_prefix_or_substring() {
        let table = table_with(&["find / -name x", "grep foo bar"]);
        assert_eq!(table.lookup("%1"), Some(1));
        assert_eq!(table.lookup("1"), Some(1));
        assert_eq!(table.lookup("%grep"), Some(2));
        assert_eq!(table.lookup("%?foo"), Some(2));
        assert_eq!(table.lookup("%nosuch"), None);
        assert_eq!(table.lookup("%9"), None);
    }

    /// Removing the current job promotes the previous one, so `%%` never dangles.
    #[test]
    fn removing_the_current_job_promotes_the_previous() {
        let mut table = table_with(&["a", "b"]);
        table.remove(2);
        assert_eq!(table.current_id(), Some(1));
        assert_eq!(table.previous_id(), None);
    }

    /// Job numbers are reused once the table empties: a session that backgrounds one job at a
    /// time should keep calling it `%1`.
    #[test]
    fn job_numbers_restart_when_the_table_empties() {
        let mut table = table_with(&["a"]);
        table.remove(1);
        let id = table.add_background(Pid::from_raw(9), vec![Pid::from_raw(9)], "b".into());
        assert_eq!(id, 1);
    }

    /// R7.5's half of the bargain: a status the reaper collected is still available once, by pid.
    ///
    /// Once, not twice: `bash --posix` — the oracle the differential corpus runs against — answers
    /// a repeated `wait $p` with 127. Default bash keeps the status instead, so this assertion is
    /// a deliberate choice of POSIX over bash, not an accident.
    #[test]
    fn a_reaped_status_is_available_to_wait_exactly_once() {
        let mut table = JobTable::default();
        table.remember_status(Pid::from_raw(42), 7);
        assert_eq!(table.take_status(Pid::from_raw(42)), Some(7));
        assert_eq!(table.take_status(Pid::from_raw(42)), None);
    }

    /// The remembered-status list is bounded, or a shell that never waits leaks one entry per job.
    #[test]
    fn remembered_statuses_are_capped() {
        let mut table = JobTable::default();
        for pid in 0..(super::REMEMBERED_STATUSES as i32 + 10) {
            table.remember_status(Pid::from_raw(pid + 1), pid);
        }
        assert_eq!(table.statuses.len(), super::REMEMBERED_STATUSES);
        // The oldest were dropped, the newest kept.
        assert_eq!(table.take_status(Pid::from_raw(1)), None);
        assert!(
            table
                .take_status(Pid::from_raw(super::REMEMBERED_STATUSES as i32 + 10))
                .is_some()
        );
    }
}

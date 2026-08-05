//! What the shell *tells the user* about its jobs.
//!
//! Split from [`super::table`] because the two answer different questions. The table is
//! bookkeeping — which jobs exist, what state each is in, which one `%%` means. This is the
//! rendering and the announcing: the exact column layout `jobs` prints, and the rule about who is
//! allowed to print a `Done` line so the user never sees it twice.
//!
//! Both halves of that rule are here on purpose. The layout is only right if `describe` and the
//! change notice agree, and they only agree if one function produces both.

use super::table::{Job, JobState, JobTable};

/// One line of `jobs` output, and the notice printed when a job changes state.
///
/// bash's column layout, because that is what scripts and eyes both expect: the state is padded
/// to 27 columns and a job running detached keeps the `&` the user typed, so the line reads
/// `[1]+  Running                    sleep 1 &`.
pub fn describe(job: &Job, marker: char) -> String {
    let state = match job.state {
        JobState::Running => "Running".to_string(),
        JobState::Stopped => "Stopped".to_string(),
        JobState::Completed(0) => "Done".to_string(),
        JobState::Completed(code) => format!("Exit {}", code),
    };
    // The `&` says "still running detached", not "was started with `&`": bash prints
    // `sleep 1 &` while it runs and plain `sleep 1` on the `Done` line, because a finished job is
    // not in the background any more.
    let amp = match job.background && job.state == JobState::Running {
        true => " &",
        false => "",
    };
    format!(
        "[{}]{}  {:<width$}{}{}",
        job.id,
        marker,
        state,
        job.command,
        amp,
        width = STATE_COLUMN
    )
}

/// Width of the state column in `jobs` output, measured against bash 5: `Done` is followed by 23
/// spaces, `Running` by 20, so the command always starts in column 34.
const STATE_COLUMN: usize = 27;

/// Tell an interactive user what changed, and forget the jobs that have nothing left to say.
///
/// Exactly one thing may report a finished job, or the user sees the `Done` line twice. bash
/// resolves that by having whichever printer gets there first delete the entry, and so does this:
/// an interactive shell announces a job at the command boundary after it ends and drops it in the
/// same breath, which leaves the `jobs` builtin nothing to repeat.
///
/// A non-interactive shell announces nothing — the notice would land in the middle of a script's
/// output and inside every `$( )` that contains a `&` — so there the entry is left alone and
/// `jobs` (or `wait`) is the one printer, reporting it once and removing it.
///
/// A *stopped* job is announced but kept: `fg` and `bg` exist to act on it, and `jobs` lists it
/// every time until it is resumed.
pub(super) fn announce_changes(jobs: &mut JobTable) {
    if !crate::exec::pipeline::is_interactive() {
        jobs.forget_stale_completions();
        return;
    }
    let ids: Vec<usize> = jobs.jobs().iter().map(|job| job.id).collect();
    let mut retire = Vec::new();

    for id in ids {
        let marker = jobs.marker(id);
        let Some(job) = jobs.get_mut(id) else {
            continue;
        };
        if job.notified {
            continue;
        }
        let ended = matches!(job.state, JobState::Completed(_));
        if !ended && job.state != JobState::Stopped {
            continue;
        }
        job.notified = true;
        let line = describe(job, marker);
        eprintln!("{}", line);
        if ended {
            // `on-job-finish`, beside the `[1]+ Done` line rather than instead of it, and only for
            // a job that *ended* — a stopped job is announced here too but it has not finished,
            // and `fg` may yet resume it.
            let status = match job.state {
                JobState::Completed(code) => code,
                _ => 0,
            };
            crate::lua::engine::fire_at_here(
                crate::lua::api::hooks::at::JOB_FINISH,
                &[
                    ("id", &id.to_string()),
                    ("pid", &job.pgid.as_raw().to_string()),
                    ("text", &job.command),
                    ("status", &status.to_string()),
                ],
            );
            retire.push(id);
        }
    }
    for id in retire {
        jobs.remove(id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nix::unistd::Pid;

    fn running(command: &str) -> Job {
        Job {
            id: 1,
            pgid: Pid::from_raw(5),
            pids: vec![Pid::from_raw(5)],
            ended: Vec::new(),
            command: command.to_string(),
            state: JobState::Running,
            notified: false,
            background: false,
        }
    }

    /// The line `jobs` prints, and the notice a finished job produces.
    ///
    /// Every column here was measured against bash 5 rather than guessed: `bash -c 'sleep 1 &
    /// jobs'` starts the command in column 34, and the state column used to be three characters
    /// short of that.
    #[test]
    fn a_job_line_reads_like_bashs() {
        let job = running("sleep 10");
        assert_eq!(
            describe(&job, '+'),
            "[1]+  Running                    sleep 10"
        );

        let done = Job {
            state: JobState::Completed(3),
            ..job.clone()
        };
        assert_eq!(
            describe(&done, '-'),
            "[1]-  Exit 3                     sleep 10"
        );

        let finished = Job {
            state: JobState::Completed(0),
            ..job.clone()
        };
        assert_eq!(
            describe(&finished, ' '),
            "[1]   Done                       sleep 10"
        );

        let stopped = Job {
            state: JobState::Stopped,
            ..job.clone()
        };
        assert_eq!(
            describe(&stopped, ' '),
            "[1]   Stopped                    sleep 10"
        );
    }

    /// A job *running* detached carries the `&`; one stopped out of the foreground never had it,
    /// and one that has finished has stopped being in the background. All three measured against
    /// bash 5, which prints `sleep 1 &` for `Running` and a bare `sleep 1` on the `Done` line.
    #[test]
    fn only_a_running_detached_job_shows_an_ampersand() {
        let detached = Job {
            background: true,
            ..running("sleep 10")
        };
        assert_eq!(
            describe(&detached, '+'),
            "[1]+  Running                    sleep 10 &"
        );

        let finished = Job {
            state: JobState::Completed(0),
            ..detached.clone()
        };
        assert_eq!(
            describe(&finished, '+'),
            "[1]+  Done                       sleep 10"
        );

        let stopped = Job {
            state: JobState::Stopped,
            ..running("vim")
        };
        assert_eq!(
            describe(&stopped, '+'),
            "[1]+  Stopped                    vim"
        );
    }
}

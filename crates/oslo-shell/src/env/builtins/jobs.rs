//! Job-control builtins: `jobs`, `fg`, `bg`, `disown` — and `wait`, which is the one of them a
//! script ever runs.
//!
//! None of them owns any state. The job table, the terminal handover and the reaping discipline
//! all live in [`crate::exec::job`], because a builtin is the wrong place to decide who owns the
//! terminal: `fg` and the evaluator's own foreground wait have to agree about that, and two
//! implementations of "hand the terminal over, take it back with SIGTTOU blocked" is one too
//! many. What is left here is argument handling and output, which is all these four are.
//!
//! `wait` is in its own file for a different reason: it is the only one of the five with real
//! behaviour of its own — an operand grammar, `-n`, and a status to compute — and it is the only
//! one that works without a terminal.

mod wait;

pub use wait::builtin_wait;

use crate::env::scope::Environment;
use crate::exec::job::{
    Job, JobState, JobTable, continue_in_background, describe, foreground_job, job_control_active,
    with_jobs,
};
use oslo_base::error::Result;

/// `jobs [-lnprs] [jobspec …]` — report what the shell still has running.
pub fn builtin_jobs(_env: &mut Environment, args: &[String]) -> Result<i32> {
    let mut long = false;
    let mut pids_only = false;
    let mut running_only = false;
    let mut stopped_only = false;
    let (flags, operands) = split_operands(&args[1..]);
    for c in flags {
        match c {
            'l' => long = true,
            'p' => pids_only = true,
            'r' => running_only = true,
            's' => stopped_only = true,
            // `-n` is "only jobs whose state changed since the last report". The table tracks
            // that as `notified`, but the shell's own change notices already consume it, so `-n`
            // lists everything rather than silently listing nothing: an honest superset.
            'n' => {}
            other => {
                eprintln!("oslo: jobs: -{}: invalid option", other);
                eprintln!("jobs: usage: jobs [-lnprs] [jobspec ...]");
                return Ok(2);
            }
        }
    }

    let (lines, finished) = with_jobs(|jobs| {
        let selected = match select(jobs, &operands, "jobs") {
            Ok(ids) => ids,
            Err(status) => return (Err(status), Vec::new()),
        };
        let mut lines = Vec::new();
        let mut finished = Vec::new();
        for id in selected {
            let marker = jobs.marker(id);
            let Some(job) = jobs.get(id) else { continue };
            if !wanted(job, running_only, stopped_only) {
                continue;
            }
            match pids_only {
                true => lines.extend(job.pids.iter().map(|pid| pid.to_string())),
                false => lines.push(long_form(job, marker, long)),
            }
            if matches!(job.state, JobState::Completed(_)) {
                finished.push(id);
            }
        }
        (Ok(lines), finished)
    });

    let lines = match lines {
        Ok(lines) => lines,
        Err(status) => return Ok(status),
    };
    for line in lines {
        println!("{}", line);
    }
    // A finished job is reported exactly once and then forgotten, as bash does: the second
    // `jobs` after a background command ends prints nothing at all.
    with_jobs(|jobs| {
        for id in finished {
            jobs.remove(id);
        }
    });
    Ok(0)
}

/// Whether a job passes the `-r`/`-s` filters.
fn wanted(job: &Job, running_only: bool, stopped_only: bool) -> bool {
    match job.state {
        JobState::Running => !stopped_only,
        JobState::Stopped => !running_only,
        JobState::Completed(_) => !running_only && !stopped_only,
    }
}

/// A `jobs` line, with the process group spliced in for `-l`.
fn long_form(job: &Job, marker: char, long: bool) -> String {
    let line = describe(job, marker);
    if !long {
        return line;
    }
    // bash's `-l` spends the two spaces after the marker on the process group instead:
    // `[1]+  Running …` becomes `[1]+ 776270 Running …`, with the state column unmoved.
    match line.split_once("  ") {
        Some((head, tail)) => format!("{} {} {}", head, job.pgid, tail),
        None => line,
    }
}

/// `fg [jobspec]` — bring a job to the foreground and wait for it.
pub fn builtin_fg(_env: &mut Environment, args: &[String]) -> Result<i32> {
    let Some(id) = one_job(&args[1..], "fg")? else {
        return Ok(1);
    };
    // Announced before the handover, because after it the job owns the terminal and anything the
    // shell writes would interleave with the job's own output.
    if let Some(command) = with_jobs(|jobs| jobs.get(id).map(|j| j.command.clone())) {
        println!("{}", command);
    }
    Ok(foreground_job(id).unwrap_or(NO_SUCH_JOB))
}

/// `bg [jobspec …]` — continue stopped jobs in the background.
pub fn builtin_bg(_env: &mut Environment, args: &[String]) -> Result<i32> {
    if !job_control_active() {
        eprintln!("oslo: bg: no job control");
        return Ok(1);
    }
    let (_, operands) = split_operands(&args[1..]);
    let ids = match with_jobs(|jobs| select(jobs, &operands, "bg")) {
        Ok(ids) => ids,
        Err(status) => return Ok(status),
    };
    if ids.is_empty() {
        eprintln!("oslo: bg: current: no such job");
        return Ok(1);
    }
    for id in ids {
        if let Some(line) = with_jobs(|jobs| {
            let marker = jobs.marker(id);
            jobs.get(id)
                .map(|job| format!("[{}]{} {} &", job.id, marker, job.command))
        }) {
            println!("{}", line);
        }
        continue_in_background(id);
    }
    Ok(0)
}

/// `disown [-ahr] [jobspec …]` — stop tracking a job, so the shell forgets it exists.
pub fn builtin_disown(_env: &mut Environment, args: &[String]) -> Result<i32> {
    let mut all = false;
    let mut running_only = false;
    let mut keep_listed = false;
    let (flags, operands) = split_operands(&args[1..]);
    for c in flags {
        match c {
            'a' => all = true,
            'r' => running_only = true,
            'h' => keep_listed = true,
            other => {
                eprintln!("oslo: disown: -{}: invalid option", other);
                eprintln!("disown: usage: disown [-h] [-ar] [jobspec ... | pid ...]");
                return Ok(2);
            }
        }
    }

    let targets = with_jobs(|jobs| match all || (running_only && operands.is_empty()) {
        true => Ok(jobs
            .jobs()
            .iter()
            .filter(|j| !running_only || j.state == JobState::Running)
            .map(|j| j.id)
            .collect()),
        false => select(jobs, &operands, "disown"),
    });
    let targets = match targets {
        Ok(ids) => ids,
        Err(status) => return Ok(status),
    };
    if targets.is_empty() {
        eprintln!("oslo: disown: current: no such job");
        return Ok(1);
    }

    with_jobs(|jobs| {
        for id in targets {
            // `-h` keeps the job listed and only exempts it from the shell's exit-time SIGHUP;
            // without it the job leaves the table altogether. Marking it as already notified is
            // what that exemption amounts to here — the shell stops reporting on it.
            match keep_listed {
                true => {
                    if let Some(job) = jobs.get_mut(id) {
                        job.notified = true;
                    }
                }
                false => {
                    jobs.remove(id);
                }
            }
        }
    });
    Ok(0)
}

/// Status for a job spec that names nothing, in the builtins that are not `wait`.
const NO_SUCH_JOB: i32 = 1;

/// Split a builtin's arguments into option letters and operands.
///
/// A word starting with `%` is always an operand: `%-` is the previous job, not a `-` option, and
/// reading it as one is how job specs get eaten by a naive getopt loop.
fn split_operands(args: &[String]) -> (Vec<char>, Vec<String>) {
    let mut flags = Vec::new();
    let mut it = args.iter();
    let mut operands = Vec::new();
    for arg in it.by_ref() {
        if arg == "--" {
            break;
        }
        if arg.len() < 2 || !arg.starts_with('-') || arg.starts_with('%') {
            operands.push(arg.clone());
            break;
        }
        flags.extend(arg[1..].chars());
    }
    operands.extend(it.cloned());
    (flags, operands)
}

/// The single job `fg` acts on, with the diagnostics both failure modes need.
///
/// `Ok(None)` means a message has already been printed and the caller should fail.
fn one_job(args: &[String], name: &str) -> Result<Option<usize>> {
    if !job_control_active() {
        // Not a stub: with job control off the job shares the shell's process group and its
        // terminal, so there is nothing to hand over and nothing to take back. bash says exactly
        // this and fails, and a script that sees success here would wait on a job that is not
        // in the foreground at all.
        eprintln!("oslo: {}: no job control", name);
        return Ok(None);
    }
    let (_, operands) = split_operands(args);
    let ids = match with_jobs(|jobs| select(jobs, &operands, name)) {
        Ok(ids) => ids,
        Err(_) => return Ok(None),
    };
    match ids.first() {
        Some(id) => Ok(Some(*id)),
        None => {
            eprintln!("oslo: {}: current: no such job", name);
            Ok(None)
        }
    }
}

/// Resolve operands to job ids, defaulting to the current job when there are none.
///
/// `Err(status)` means a diagnostic has already been printed. The status is 1 rather than
/// `wait`'s 127: bash's `fg %9` and `disown %9` both fail with 1, and only `wait` uses the
/// "no such child" number.
fn select(
    jobs: &JobTable,
    operands: &[String],
    name: &str,
) -> std::result::Result<Vec<usize>, i32> {
    if operands.is_empty() {
        return Ok(jobs.current_id().into_iter().collect());
    }
    let mut out = Vec::new();
    for operand in operands {
        match jobs.lookup(operand) {
            Some(id) => out.push(id),
            None => {
                eprintln!("oslo: {}: {}: no such job", name, operand);
                return Err(NO_SUCH_JOB);
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nix::unistd::Pid;

    fn args(words: &[&str]) -> Vec<String> {
        words.iter().map(|s| s.to_string()).collect()
    }

    /// Every builtin here has to survive an empty table, which is the state a script is in.
    #[test]
    fn jobs_with_an_empty_table_succeeds_quietly() {
        let mut env = Environment::new();
        assert_eq!(builtin_jobs(&mut env, &args(&["jobs"])).unwrap(), 0);
    }

    #[test]
    fn an_unknown_option_is_a_usage_error() {
        let mut env = Environment::new();
        assert_eq!(builtin_jobs(&mut env, &args(&["jobs", "-Z"])).unwrap(), 2);
        assert_eq!(
            builtin_disown(&mut env, &args(&["disown", "-Z"])).unwrap(),
            2
        );
    }

    /// Non-interactive is the only mode a script ever runs in, and bash fails there rather than
    /// half-working. `fg` reporting success without a terminal would be the worse answer.
    #[test]
    fn fg_and_bg_refuse_without_job_control() {
        let mut env = Environment::new();
        assert_eq!(builtin_fg(&mut env, &args(&["fg"])).unwrap(), 1);
        assert_eq!(builtin_bg(&mut env, &args(&["bg", "%1"])).unwrap(), 1);
    }

    /// A `%` word is an operand however much it looks like an option cluster.
    #[test]
    fn a_job_spec_is_never_parsed_as_an_option() {
        let (flags, operands) = split_operands(&args(&["-l", "%-"]));
        assert_eq!(flags, vec!['l']);
        assert_eq!(operands, vec!["%-".to_string()]);

        let (flags, operands) = split_operands(&args(&["%1", "%2"]));
        assert!(flags.is_empty());
        assert_eq!(operands, vec!["%1".to_string(), "%2".to_string()]);

        let (flags, operands) = split_operands(&args(&["--", "-r"]));
        assert!(flags.is_empty());
        assert_eq!(operands, vec!["-r".to_string()]);
    }

    /// The table is process-wide, so these exercise it through a table of their own rather than
    /// through the builtins, which would race every other test in the binary.
    #[test]
    fn operands_resolve_to_job_ids_and_a_missing_one_fails() {
        let mut jobs = JobTable::default();
        jobs.add_background(
            Pid::from_raw(11),
            vec![Pid::from_raw(11)],
            "sleep 30".into(),
        );
        jobs.add_background(Pid::from_raw(12), vec![Pid::from_raw(12)], "grep x".into());

        assert_eq!(select(&jobs, &args(&["%1"]), "fg"), Ok(vec![1]));
        assert_eq!(select(&jobs, &args(&["%grep"]), "fg"), Ok(vec![2]));
        assert_eq!(select(&jobs, &args(&["%1", "%2"]), "fg"), Ok(vec![1, 2]));
        assert_eq!(select(&jobs, &args(&["%9"]), "fg"), Err(NO_SUCH_JOB));
        // No operands means the current job — what a bare `fg`/`bg`/`disown` acts on.
        assert_eq!(select(&jobs, &[], "fg"), Ok(vec![2]));
        assert_eq!(select(&JobTable::default(), &[], "fg"), Ok(vec![]));
    }

    #[test]
    fn the_r_and_s_filters_pick_out_running_and_stopped_jobs() {
        let mut jobs = JobTable::default();
        jobs.add_background(Pid::from_raw(21), vec![Pid::from_raw(21)], "a".into());
        let running = jobs.get(1).expect("job 1");
        assert!(wanted(running, true, false));
        assert!(!wanted(running, false, true));

        jobs.add_stopped(Pid::from_raw(22), vec![Pid::from_raw(22)], "b".into());
        let stopped = jobs.get(2).expect("job 2");
        assert!(!wanted(stopped, true, false));
        assert!(wanted(stopped, false, true));
    }

    /// `-l` inserts the process group without disturbing the column the command sits in.
    #[test]
    fn the_long_form_shows_the_process_group() {
        let mut jobs = JobTable::default();
        jobs.add_background(
            Pid::from_raw(4242),
            vec![Pid::from_raw(4242)],
            "sleep 30".into(),
        );
        let job = jobs.get(1).expect("job 1");
        // The state column is 27 wide and a running background job keeps its `&`, both measured
        // against `bash -c 'sleep 1 & jobs -l'`. This used to assert a 24-wide column and no `&`.
        assert_eq!(
            long_form(job, '+', false),
            "[1]+  Running                    sleep 30 &"
        );
        assert_eq!(
            long_form(job, '+', true),
            "[1]+ 4242 Running                    sleep 30 &"
        );
    }
}

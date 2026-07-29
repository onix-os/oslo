//! Process groups and zombie reaping — the parts of job control a test can see without a pty.
//!
//! R7.1 and R7.4. Everything the terminal arbitrates — `tcsetpgrp`, Ctrl-C reaching only the
//! foreground job, Ctrl-Z parking one — needs a controlling terminal and is verified by hand.
//! What survives without one is still the load-bearing half: *which process group each child ends
//! up in*, and whether the shell ever collects the children it started. Both are readable from an
//! ordinary `oslo -c` run.
//!
//! Every test here asserts that its probe returned data before drawing a conclusion from it.
//! That is not defensive padding: when these read `/proc/<pid>/stat` on a machine that had no
//! `/proc`, `awk` printed nothing, the comparisons became `"" == ""`, and three of them passed
//! while measuring nothing at all. A test that cannot fail is worse than an absent one.
//!
//! A process cannot report its own group through the shell's `$$` (that is the shell's group, not
//! the child's), so where a *child's* group is the subject the script runs `sh -c 'ps -o pgid= -p
//! $$'` — an external command reporting on itself, which is exactly the process whose placement
//! is in question.

mod common;

use common::{oslo_bin, run};
use std::process::{Command, Stdio};

/// R7.4, the finding as reported: five background jobs left five `Z`-state children behind for
/// the shell's lifetime, because nothing ever waited for them.
///
/// `ps` runs after the jobs have had time to exit *and* after several later commands, since
/// reaping is opportunistic and happens at command boundaries.
#[test]
fn background_jobs_leave_no_zombies() {
    let r = run("sleep 5 &
         keep=$!
         for i in 1 2 3 4 5 6 7 8 9 10; do sleep 0 & done
         sleep 0.4
         :
         :
         ps -A -o ppid=,stat= | awk -v p=$$ '$1 == p { print $2 }'
         kill $keep");
    assert_child_states_were_observed(&r.stdout, &r.stderr);
    let zombies = child_states(&r.stdout)
        .filter(|s| s.starts_with('Z'))
        .count();
    assert_eq!(
        zombies, 0,
        "ten background jobs left {} defunct children:\n{}",
        zombies, r.stdout
    );
}

/// The same, at a scale that would be obvious in `ps`: twenty jobs, none of them left behind.
#[test]
fn twenty_background_jobs_are_all_collected() {
    let r = run("sleep 5 &
         keep=$!
         i=0
         while [ $i -lt 20 ]; do
             sleep 0 &
             i=$((i + 1))
         done
         sleep 0.5
         :
         ps -A -o ppid=,stat= | awk -v p=$$ '$1 == p { print $2 }'
         kill $keep");
    assert_child_states_were_observed(&r.stdout, &r.stderr);
    let zombies = child_states(&r.stdout)
        .filter(|s| s.starts_with('Z'))
        .count();
    assert_eq!(zombies, 0, "twenty background jobs left {zombies} behind");
}

/// The `ps | awk` probe reports one line per child of the shell.
fn child_states(stdout: &str) -> impl Iterator<Item = &str> {
    stdout.lines().map(str::trim).filter(|l| !l.is_empty())
}

/// The zombie tests keep one `sleep` alive precisely so the probe has something to find. If it
/// finds nothing, `ps` or `awk` did not work here and a count of zero zombies means nothing —
/// which is how these two once passed while measuring nothing.
fn assert_child_states_were_observed(stdout: &str, stderr: &str) {
    assert!(
        child_states(stdout).next().is_some(),
        "the ps/awk probe listed no children at all, so it cannot show the absence of zombies; \
         stderr: {stderr}"
    );
}

/// R7.1: a background job leads a process group of its own, so a signal aimed at it — or at the
/// terminal's foreground group — cannot reach the other.
///
/// This is the half of R7.1 that holds with or without a terminal, and it is what makes
/// `kill %1` a group operation rather than a single-process one.
#[test]
fn a_background_job_leads_its_own_process_group() {
    let r = run(r#"sleep 5 &
           bg=$!
           ps -o pid=,pgid= -p $bg
           kill $bg"#);
    let mut fields = r.out().split_whitespace();
    let pid: i32 = fields
        .next()
        .unwrap_or_else(|| panic!("ps reported no pid; stderr: {}", r.stderr))
        .parse()
        .expect("numeric pid");
    let pgrp: i32 = fields
        .next()
        .unwrap_or_else(|| panic!("ps reported no pgid; stderr: {}", r.stderr))
        .parse()
        .expect("numeric pgrp");
    assert_eq!(
        pid, pgrp,
        "a background job must lead its own group, got pid {pid} in group {pgrp}"
    );
}

/// Two background jobs get two *different* groups, or the isolation is only nominal.
#[test]
fn two_background_jobs_do_not_share_a_group() {
    let r = run(r#"sleep 5 & a=$!
           sleep 5 & b=$!
           ps -o pgid= -p $a
           ps -o pgid= -p $b
           kill $a $b"#);
    let groups: Vec<&str> = r.out().split_whitespace().collect();
    assert_eq!(groups.len(), 2, "expected two groups, got: {}", r.out());
    assert_ne!(groups[0], groups[1], "both jobs landed in one group");
}

/// The other side of R7.1, and the regression it is easiest to introduce: **without** job control
/// a foreground command must stay in the shell's process group.
///
/// That group is what the tty driver signals, so a `sleep` moved out of it would go on sleeping
/// through the Ctrl-C that was meant to kill it. bash makes the same distinction — process groups
/// per job are an interactive shell's business — which is why a script's behaviour is unchanged.
#[test]
fn a_foreground_command_stays_in_the_shells_group_without_job_control() {
    // An external command reporting its *own* group — the process whose placement is the subject.
    let script = r#"sh -c 'ps -o pgid= -p $$'"#;
    let output = Command::new(oslo_bin())
        .arg("-c")
        .arg(script)
        .stdin(Stdio::null())
        .output()
        .expect("spawn oslo");
    let child_pgrp = String::from_utf8_lossy(&output.stdout).trim().to_string();

    // The pgid of the oslo process itself, read the same way, from the same non-interactive
    // invocation style.
    let own = Command::new(oslo_bin())
        .arg("-c")
        .arg(r#"ps -o pgid= -p $$"#)
        .stdin(Stdio::null())
        .output()
        .expect("spawn oslo");
    let shell_pgrp = String::from_utf8_lossy(&own.stdout).trim().to_string();

    assert!(
        !child_pgrp.is_empty() && !shell_pgrp.is_empty(),
        "ps reported no group (child {child_pgrp:?}, shell {shell_pgrp:?}); \
         comparing two empty strings would pass while testing nothing"
    );
    assert_eq!(
        child_pgrp, shell_pgrp,
        "a foreground command was moved out of the shell's process group with job control off"
    );
}

/// Pipeline stages follow the same rule: without job control they stay where the shell is, so a
/// terminal signal still reaches all of them.
#[test]
fn pipeline_stages_stay_in_the_shells_group_without_job_control() {
    let r = run(r#"sh -c 'ps -o pgid= -p $$' | cat; ps -o pgid= -p $$"#);
    let lines: Vec<&str> = r.out().lines().map(str::trim).collect();
    assert_eq!(
        lines.len(),
        2,
        "expected two pgids, got {:?}; stderr: {}",
        r.out(),
        r.stderr
    );
    assert_eq!(
        lines[0], lines[1],
        "a pipeline stage left the shell's group"
    );
}

/// `$!` names the job the shell actually started, and the job outlives the command that started
/// it — the precondition for everything `wait`, `jobs` and `kill %n` do.
#[test]
fn the_background_pid_names_a_live_process() {
    // `kill -0` asks the kernel directly whether the pid exists, which is the question; testing
    // for `/proc/$!` asks whether procfs is mounted as well.
    let r = run(r#"sleep 5 &
           bg=$!
           kill -0 $bg && echo alive
           kill $bg"#);
    assert_eq!(r.out(), "alive", "stderr: {}", r.stderr);
}

//! The `time` keyword (R8.7).
//!
//! `time` used to be parsed and thrown away, so `time sleep 0.2` printed nothing and reported 0.
//! What matters about the fix is not the numbers — they are not reproducible — but *where they
//! go*: three lines on stderr, nothing added to stdout, and the pipeline's own status untouched.
//! An end-to-end suite is the only place that can see all three at once.

mod common;

use common::run;

/// The report's shape, as bash writes it: a blank line, then `real`/`user`/`sys`, tab-separated.
fn assert_timing_report(stderr: &str) {
    let lines: Vec<&str> = stderr.lines().collect();
    assert_eq!(
        lines.len(),
        4,
        "expected a blank line and three timing lines, got {stderr:?}"
    );
    assert_eq!(lines[0], "", "bash separates the report with a blank line");
    for (line, clock) in lines[1..].iter().zip(["real", "user", "sys"]) {
        let (name, value) = line
            .split_once('\t')
            .unwrap_or_else(|| panic!("{line:?} is not `<clock>\\t<value>`"));
        assert_eq!(name, clock, "clocks are reported in bash's order");
        assert!(
            value.ends_with('s') && value.contains('m') && value.contains('.'),
            "{value:?} is not bash's `<min>m<sec>.<millis>s`"
        );
    }
}

#[test]
fn time_reports_three_clocks_on_stderr_and_leaves_stdout_alone() {
    let r = run("time echo hi");
    assert_eq!(r.stdout, "hi\n", "the report must not reach stdout");
    assert_timing_report(&r.stderr);
    assert_eq!(r.status, 0);
}

/// The pipeline's status is its own: `time` is a report, not a command that can succeed.
#[test]
fn time_preserves_a_failing_status() {
    let r = run("time false");
    assert_eq!(r.stdout, "");
    assert_eq!(r.status, 1);
    assert_timing_report(&r.stderr);
}

/// `exit` unwinds as an error carrying its code, so the timer has to be stopped on that path too
/// — bash prints the report for `time exit 3` and *then* exits 3.
#[test]
fn time_reports_even_when_the_pipeline_exits() {
    let r = run("time exit 3");
    assert_eq!(r.status, 3);
    assert_timing_report(&r.stderr);
}

/// The whole point of stderr: a timed command inside `$( )` must not have its timing captured.
#[test]
fn time_does_not_pollute_a_command_substitution() {
    let r = run("x=$(time echo captured); echo \"[$x]\"");
    assert_eq!(r.stdout, "[captured]\n");
    assert_timing_report(&r.stderr);
}

/// `time a | b` times the pipeline, not its first stage — one report, not two.
#[test]
fn time_covers_a_whole_pipeline() {
    let r = run("time { echo a; echo b; } | cat");
    assert_eq!(r.stdout, "a\nb\n");
    assert_timing_report(&r.stderr);
}

/// Without the keyword nothing is measured and nothing is printed, however slow the command is.
#[test]
fn an_untimed_pipeline_stays_silent() {
    let r = run("echo hi");
    assert_eq!(r.stdout, "hi\n");
    assert_eq!(r.stderr, "");
}

/// `real` measures wall clock, so a sleep has to show up in it — a report of all zeroes would
/// mean the timer was started and stopped around nothing.
#[test]
fn real_time_reflects_a_sleep() {
    let r = run("time sleep 0.2");
    assert_timing_report(&r.stderr);
    let real = r
        .stderr
        .lines()
        .find_map(|l| l.strip_prefix("real\t"))
        .expect("a real line");
    let secs: f64 = real
        .trim_start_matches("0m")
        .trim_end_matches('s')
        .parse()
        .unwrap_or_else(|e| panic!("{real:?}: {e}"));
    assert!(secs >= 0.2, "sleep 0.2 was reported as {real}");
}

/// `getrusage` is cumulative over the shell's whole life, so each report is a difference. Without
/// that, the second `time` in a script inherits everything the first one measured.
#[test]
fn a_later_timing_does_not_inherit_an_earlier_one() {
    let r = run("time sleep 0.3; time true");
    let reals: Vec<&str> = r
        .stderr
        .lines()
        .filter_map(|l| l.strip_prefix("real\t"))
        .collect();
    assert_eq!(
        reals.len(),
        2,
        "one report per timed pipeline: {:?}",
        r.stderr
    );
    assert!(
        reals[1].starts_with("0m0.0"),
        "the second pipeline was charged for the first: {}",
        reals[1]
    );
}

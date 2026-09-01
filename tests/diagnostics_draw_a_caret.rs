//! The other face: what a person at a terminal sees.
//!
//! `tests/diagnostics_stay_plain.rs` holds the rule — a pipe sees what it always saw. This holds
//! the reason there is a second face at all, and the two together are the whole contract.
//!
//! # Why `OSLO_DIAG=always` and not a pty
//!
//! Because the thing under test is the *report*, not the terminal handling. `enabled()` reads the
//! variable before it asks about a tty precisely so the drawing can be tested without one — which
//! makes these ordinary, fast, non-flaky tests rather than forty lines of pty driving per case.
//!
//! That the variable is read at all is checked here too, since a report drawn only under a pty
//! would be a report nothing could test.
//!
//! # Skipped without the feature
//!
//! `diagnostics` is off in a default build, where `diag_stub` draws nothing and every assertion
//! here would fail for the right reason. The `cfg` is on the whole file rather than on each test,
//! so the suite is empty rather than red.

#![cfg(feature = "diagnostics")]

mod common;

use common::oslo_bin;
use std::process::{Command, Stdio};

/// stderr of `oslo -c script`, with the reports forced on.
fn drawn(script: &str) -> String {
    let output = Command::new(oslo_bin())
        .arg("-c")
        .arg(script)
        .env("OSLO_DIAG", "always")
        .env("NO_COLOR", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .output()
        .expect("spawn oslo");
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// **The one-liner is the report's first line.** Not a summary of it, not a rewording — the same
/// bytes a pipe gets, with the picture underneath. That is what makes the two faces one error.
#[test]
fn the_first_line_is_the_message_a_pipe_would_get() {
    let report = drawn("kill -s NOPE 1");
    assert_eq!(
        report.lines().next(),
        Some("oslo: kill: NOPE: invalid signal specification"),
        "{report}"
    );
}

/// ariadne opens every report with a `Kind: message` header of its own. oslo's message is already
/// the first line, so that header is dropped — and an empty `Error:` left behind would be the most
/// visible possible bug.
#[test]
fn ariadnes_own_header_is_not_left_behind() {
    let report = drawn("kill -s NOPE 1");
    assert!(!report.contains("Error:"), "{report}");
}

/// The caret goes under the word at fault, and the source line is the command as it was given.
#[test]
fn the_caret_is_under_the_word() {
    let report = drawn("kill -s NOPE 1");
    assert!(report.contains("kill -s NOPE 1"), "the words: {report}");
    assert!(report.contains("not a signal"), "the label: {report}");
    // **Measured in characters, against the line above it.** A byte offset would be wrong the
    // moment the box drawing is counted — `│` is three bytes — and hard-coding a column would pass
    // for a report pointing at the wrong word if the layout ever moved.
    let source = report
        .lines()
        .find(|line| line.contains("kill -s NOPE 1"))
        .unwrap_or_else(|| panic!("no source line in {report}"));
    let caret = report
        .lines()
        .find(|line| line.contains('┬'))
        .unwrap_or_else(|| panic!("no caret in {report}"));

    let at = |line: &str, want: char| line.chars().position(|c| c == want);
    let word = source
        .char_indices()
        .position(|(byte, _)| source[byte..].starts_with("NOPE"))
        .expect("NOPE in the source line");
    let underline = at(caret, '─').expect("the underline");
    assert_eq!(underline, word, "caret {caret:?} under source {source:?}");
    assert!(at(caret, '┬').is_some_and(|tip| tip >= word && tip < word + 4));
}

/// The help line is what the message cannot say: the shape a right answer has.
#[test]
fn the_help_line_says_what_would_have_been_right() {
    let report = drawn("kill -s NOPE 1");
    assert!(
        report.contains("a signal is a name (TERM), a number (15)"),
        "{report}"
    );
}

/// **The report a mistyped column gets is the reason the whole feature is worth having.** The
/// shell knows the answer at plan time — it refused precisely because it knew — so the help line
/// can list the columns rather than leave the reader to find out.
#[test]
fn a_mistyped_column_is_told_which_ones_there_are() {
    let report = drawn("df | cols nmae");
    assert_eq!(
        report.lines().next(),
        Some("oslo: cols: nmae: no such column"),
        "{report}"
    );
    assert!(report.contains("cols nmae"), "the words: {report}");
    assert!(report.contains("no column of that name"), "{report}");
    assert!(report.contains("the columns here are:"), "{report}");
    assert!(report.contains("filesystem"), "and names them: {report}");
}

/// The other plan-time refusal, which has nothing to add: the message already says what to do
/// instead, so there is no help line and that is deliberate rather than missing.
#[test]
fn a_column_that_is_already_there_gets_no_help_line() {
    let report = drawn("df | insert size 1");
    assert!(report.contains("already here"), "{report}");
    assert!(!report.contains("Help:"), "{report}");
}

/// A diagnostic with nothing to point at keeps its one-liner **on a terminal too**. Drawing a box
/// around `No such file or directory` would be decoration, not information.
#[test]
fn an_error_with_nothing_to_point_at_stays_one_line() {
    let report = drawn("cd /nope");
    assert_eq!(
        report.trim_end(),
        "oslo: cd: /nope: No such file or directory",
        "{report}"
    );
}

/// `OSLO_DIAG=never` turns it off where it would otherwise draw, which is the escape hatch for
/// anyone whose terminal, pager or screen-reader is worse off for the picture.
#[test]
fn never_wins_over_always() {
    let output = Command::new(oslo_bin())
        .arg("-c")
        .arg("kill -s NOPE 1")
        .env("OSLO_DIAG", "never")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .output()
        .expect("spawn oslo");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        stderr.trim_end(),
        "oslo: kill: NOPE: invalid signal specification",
        "{stderr}"
    );
}

/// `NO_COLOR` takes the colour and leaves the caret: a person who turned colour off still wants to
/// be shown *where* the error is.
#[test]
fn no_colour_still_draws() {
    let report = drawn("kill -s NOPE 1");
    assert!(!report.contains('\u{1b}'), "no escapes: {report:?}");
    assert!(report.contains('┬'), "still a caret: {report}");
}

/// The status is the status it always was, whichever face was drawn.
#[test]
fn drawing_does_not_change_the_status() {
    for (script, want) in [
        ("kill -s NOPE 1", 1),
        ("df | cols nmae", 2),
        ("cd /nope", 1),
    ] {
        let status = Command::new(oslo_bin())
            .arg("-c")
            .arg(script)
            .env("OSLO_DIAG", "always")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .expect("spawn oslo")
            .code()
            .unwrap_or(-1);
        assert_eq!(status, want, "`{script}`");
    }
}

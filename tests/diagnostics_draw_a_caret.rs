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

/// **A syntax error points into the program itself** — the only report that does, and the one that
/// looks most like a compiler's.
#[test]
fn a_syntax_error_quotes_the_failing_line() {
    let report = drawn("echo \"unterminated");
    // **The construct, named.** The vendored parser said "unterminated double quote"; rune reports
    // the opener it was still looking for the end of, which is the thing you have to go and fix.
    assert!(report.contains("this `\"` was never closed"), "{report}");
    assert!(report.contains("echo \"unterminated"), "the line: {report}");
    assert!(report.contains('┬'), "a caret: {report}");
}

/// An error about the *absence* of text carries no column, so the report points at the construct
/// that was left open instead — the first line of the chunk that failed, which is where it began.
///
/// **The one-liner is the report's first line, whatever it says.** It used to say `syntax error at
/// end of input`; rune names the construct instead. Either way a pipe gets that line and only that
/// line, and the report is the picture under it — pointing somewhere real rather than nowhere.
#[test]
fn an_error_at_end_of_input_points_at_what_was_left_open() {
    let report = drawn("if true; then");
    // The one-liner names the construct rather than the position. `at end of input` said where the
    // parser gave up; this says what it was waiting for, which is the same information from the
    // side you can act on.
    assert_eq!(
        report.lines().next(),
        Some("oslo: syntax error: this `if` was never closed"),
        "{report}"
    );
    assert!(report.contains("if true; then"), "quoted: {report}");
    assert!(report.contains("left open here"), "{report}");
}

/// **A script gets its own file, its own line number and its own source line.**
///
/// The report for a diagnostic inside a script is not the rebuilt command line: the origin already
/// names the file and the line, so the file is read and the caret goes into the code as written.
/// This is the difference between a caret and a compiler's diagnostic, and it is what makes the
/// feature worth having on a two-hundred-line script.
#[test]
fn a_script_report_names_the_file_and_the_line() {
    let dir = tempfile::tempdir().expect("tempdir");
    let script = dir.path().join("deploy.sh");
    std::fs::write(
        &script,
        "echo one\necho two\nreadonly TARGET=prod\nkill -s NOPE 1\n",
    )
    .expect("write");

    let output = Command::new(oslo_bin())
        .arg(&script)
        .env("OSLO_DIAG", "always")
        .env("NO_COLOR", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .output()
        .expect("spawn oslo");
    let report = String::from_utf8_lossy(&output.stderr);

    assert!(
        report.contains("deploy.sh:4:9"),
        "path, line and column: {report}"
    );
    assert!(
        report.contains("kill -s NOPE 1"),
        "the line as written: {report}"
    );
    assert!(
        report.contains(" 4 │"),
        "numbered by the file, not by 1: {report}"
    );
    assert!(report.contains("not a signal"), "{report}");
}

/// The line number is the file's, so a mistake on line 40 does not read as line 1.
#[test]
fn the_line_number_is_the_files_own() {
    let dir = tempfile::tempdir().expect("tempdir");
    let script = dir.path().join("long.sh");
    let mut text = "true\n".repeat(39);
    text.push_str("kill -s NOPE 1\n");
    std::fs::write(&script, text).expect("write");

    let output = Command::new(oslo_bin())
        .arg(&script)
        .env("OSLO_DIAG", "always")
        .env("NO_COLOR", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .output()
        .expect("spawn oslo");
    let report = String::from_utf8_lossy(&output.stderr);
    assert!(report.contains("long.sh:40:9"), "{report}");
    assert!(report.contains("40 │"), "{report}");
}

/// **A sourced Lua file gets the same treatment.** Lua names the line it raised on, and the text it
/// raised in is the file that was just read — so a `.lua` sourced from a shell script reports as
/// precisely as the shell script does.
#[test]
fn a_lua_error_quotes_the_line_it_raised_on() {
    let dir = tempfile::tempdir().expect("tempdir");
    let lua = dir.path().join("broken.lua");
    std::fs::write(&lua, "local x = 1\nlocal t = nil\nprint(t.field)\n").expect("write");

    let output = Command::new(oslo_bin())
        .arg("-c")
        .arg(format!("source {}", lua.display()))
        .env("OSLO_DIAG", "always")
        .env("NO_COLOR", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .output()
        .expect("spawn oslo");
    let report = String::from_utf8_lossy(&output.stderr);

    assert!(
        report.contains("broken.lua:3"),
        "the file and line: {report}"
    );
    assert!(report.contains("print(t.field)"), "the line: {report}");
    assert!(report.contains("raised here"), "{report}");
    assert!(report.contains(" 3 │"), "{report}");
}

/// **`command not found` is the one this whole sweep existed to catch.** It lives outside the
/// builtins, so the first version of the source scan never saw it and it went unconverted through
/// six commits.
#[test]
fn a_missing_command_gets_a_caret() {
    let report = drawn("nosuchprogram --flag /tmp");
    assert_eq!(
        report.lines().next(),
        Some("oslo: nosuchprogram: command not found"),
        "{report}"
    );
    assert!(report.contains("nosuchprogram --flag /tmp"), "{report}");
    assert!(report.contains("no command of this name"), "{report}");
    assert!(
        report.contains("looked at aliases, functions, builtins and $PATH"),
        "{report}"
    );
}

/// stderr of `oslo -c script`, with the reports forced on **and coloured**.
fn coloured(script: &str) -> String {
    let output = Command::new(oslo_bin())
        .arg("-c")
        .arg(script)
        .env("OSLO_DIAG", "always")
        .env("CLICOLOR_FORCE", "1")
        .env_remove("NO_COLOR")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .output()
        .expect("spawn oslo");
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// **Not one palette colour reaches the report.**
///
/// ariadne paints with `Color::Red` and four `Color::Fixed(n)`, which are the *terminal's* colours
/// by number rather than colours — and a palette generator rewrites every one of them. A diagnostic
/// is the one piece of output that has to stay legible when everything else has been re-themed, so
/// the whole report is `38;2;r;g;b`.
///
/// This is the guard for an ariadne bump adding a sixth colour: the crate's palette is five
/// hard-coded constants with no setting, so a new one would arrive silently.
#[test]
fn the_report_is_painted_in_truecolour() {
    let report = coloured("kill -s NOPE 1");
    assert!(
        report.contains("\u{1b}[38;2;"),
        "it is coloured: {report:?}"
    );
    for palette in ["\u{1b}[38;5;", "\u{1b}[31m", "\u{1b}[33m", "\u{1b}[9"] {
        assert!(
            !report.contains(palette),
            "{palette:?} survived into the report: {report:?}"
        );
    }
}

/// The same for a report with a help line and one pointing into a file — the `Help:` colour and the
/// margin colour are two of the five, and only one report shows both.
#[test]
fn a_help_line_and_a_file_are_truecolour_too() {
    for script in ["kill -s NOPE 1", "df | cols nmae", "echo \"unterminated"] {
        let report = coloured(script);
        assert!(
            !report.contains("\u{1b}[38;5;") && !report.contains("\u{1b}[31m"),
            "`{script}`: {report:?}"
        );
    }
}

/// `NO_COLOR` still wins over the force, which is the whole of what that convention is for.
#[test]
fn no_colour_beats_the_force() {
    let output = Command::new(oslo_bin())
        .arg("-c")
        .arg("kill -s NOPE 1")
        .env("OSLO_DIAG", "always")
        .env("CLICOLOR_FORCE", "1")
        .env("NO_COLOR", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .output()
        .expect("spawn oslo");
    let report = String::from_utf8_lossy(&output.stderr);
    assert!(!report.contains('\u{1b}'), "{report:?}");
    assert!(report.contains('┬'), "still a caret: {report}");
}

/// **The report points at what was left open**, with a help line saying so — the one case where
/// the parser gives no position at all and the shell has to work out where to point on its own.
#[test]
fn an_unfinished_construct_is_pointed_at() {
    let dir = tempfile::tempdir().expect("tempdir");
    let script = dir.path().join("open.sh");
    std::fs::write(&script, "echo one\nif true; then\necho three\n").expect("write");

    let output = Command::new(oslo_bin())
        .arg(&script)
        .env("OSLO_DIAG", "always")
        .env("NO_COLOR", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .output()
        .expect("spawn oslo");
    let report = String::from_utf8_lossy(&output.stderr);

    assert!(
        report.contains("open.sh:2:1"),
        "the `if`'s own line: {report}"
    );
    assert!(report.contains("if true; then"), "quoted: {report}");
    assert!(report.contains("left open here"), "{report}");
    assert!(
        report.contains("still looking for the end of this"),
        "{report}"
    );
}

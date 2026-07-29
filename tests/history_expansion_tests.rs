//! End-to-end checks that history expansion happens at the prompt and *only* at the prompt.
//!
//! The expansion algorithm itself is unit-tested next to the code, where every event and word
//! designator is a table row. What no unit test can show is the wiring: that the REPL runs the
//! pass at all, that the rewritten line is what lands in history, and — the part that matters for
//! safety — that `-c` and script input never go near it. A `!` inside data must stay data.

mod common;

use std::io::Write;
use std::process::{Command, Stdio};

/// Feed `lines` to an interactive oslo and return `(stdout, stderr)`.
///
/// `-i` forces the REPL even though stdin is a pipe, which is what makes this scriptable. `HOME`
/// points at a scratch directory so the developer's own `~/.oslo_history` neither leaks into the
/// event numbering nor gets written to.
fn repl(dir: &std::path::Path, lines: &str) -> (String, String) {
    let mut child = Command::new(common::oslo_bin())
        .arg("-i")
        .current_dir(dir)
        .env("HOME", dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn oslo -i");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(lines.as_bytes())
        .expect("write script");
    let out = child.wait_with_output().expect("wait for oslo");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn the_prompt_expands_history_references() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (stdout, stderr) = repl(
        dir.path(),
        "echo alpha beta gamma\n!!\necho pre !$ post\n!echo\n^pre^POST\nexit\n",
    );

    let count = |want: &str| stdout.lines().filter(|l| *l == want).count();
    assert_eq!(
        count("alpha beta gamma"),
        2,
        "the typed line runs, then `!!` runs it again: {stdout:?}"
    );
    assert_eq!(
        count("pre gamma post"),
        2,
        "`!$` takes the previous line's last word, and `!echo` re-runs that line — the most \
         recent one starting with `echo`, not the first: {stdout:?}"
    );
    assert_eq!(
        count("POST gamma post"),
        1,
        "^pre^POST edits the previous line: {stdout:?}"
    );
    // bash echoes the rewritten line on stderr so the user sees what actually ran.
    assert!(
        stderr.contains("echo alpha beta gamma"),
        "the expansion must be echoed: {stderr:?}"
    );
}

#[test]
fn an_unresolvable_reference_runs_nothing_and_leaves_the_status_alone() {
    let dir = tempfile::tempdir().expect("tempdir");
    let (stdout, stderr) = repl(dir.path(), "true\n!nosuch\necho status=$?\nexit\n");

    assert!(
        stderr.contains("!nosuch: event not found"),
        "the reason must be reported: {stderr:?}"
    );
    assert!(
        stdout.lines().any(|l| l == "status=0"),
        "a failed expansion runs nothing, so `$?` is untouched: {stdout:?}"
    );
}

#[test]
fn history_expansion_never_reaches_a_non_interactive_shell() {
    let dir = tempfile::tempdir().expect("tempdir");
    // The security-relevant half of the feature: `-c` text and script text are data the shell was
    // handed, not something a user typed at a prompt, so a `!` in them is a literal `!`.
    for script in ["echo '!!'", "echo !!", "echo a!$b", "echo ^x^y"] {
        let run = common::run_in(dir.path(), script);
        assert_eq!(
            run.status, 0,
            "{script:?} should just run: {:?}",
            run.stderr
        );
        assert!(
            run.out().contains('!') || run.out().contains('^'),
            "{script:?} must print its `!`/`^` verbatim, got {:?}",
            run.out()
        );
    }
    assert_eq!(common::run_in(dir.path(), "echo !!").out(), "!!");
    assert_eq!(common::run_in(dir.path(), "echo ^x^y").out(), "^x^y");
}

#[test]
fn history_records_the_expanded_line_not_the_reference() {
    let dir = tempfile::tempdir().expect("tempdir");
    // `!!` twice in a row must not recall itself: the first one has to be stored as what it
    // became, or the second would resolve to the literal `!!` and be a syntax error.
    let (stdout, _) = repl(dir.path(), "echo kept\n!!\n!!\nexit\n");
    assert_eq!(
        stdout.lines().filter(|l| *l == "kept").count(),
        3,
        "each !! re-runs the stored expansion: {stdout:?}"
    );

    let saved = std::fs::read_to_string(dir.path().join(".oslo_history")).expect("history file");
    assert!(
        !saved.contains("!!"),
        "the raw reference must never be written to the history file: {saved:?}"
    );
}

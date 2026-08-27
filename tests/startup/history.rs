//! What a session remembers: the history file, its size, and what `$HISTFILE` decides.
//!
//! Split from the parent when it crossed the line limit. The division is the one the file already
//! drew with its own headings: startup and rc files there, the history store here.

use super::{out, repl, run};
use std::path::Path;

// ------------------------------------------------------------------------- R9.11: the history

fn history_lines(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.starts_with('#'))
        .map(str::to_string)
        .collect()
}

#[test]
fn history_keeps_more_than_rustylines_default_of_a_hundred() {
    let dir = tempfile::tempdir().unwrap();
    let hist = dir.path().join("hist");
    let input: String = (1..=120).map(|i| format!(": {i}\n")).collect();

    repl(&input, &[("HISTFILE", hist.to_str().unwrap())], dir.path());
    assert_eq!(history_lines(&hist).len(), 120);
}

#[test]
fn histsize_caps_the_history() {
    let dir = tempfile::tempdir().unwrap();
    let hist = dir.path().join("hist");
    repl(
        ": a\n: b\n: c\n: d\n",
        &[("HISTFILE", hist.to_str().unwrap()), ("HISTSIZE", "2")],
        dir.path(),
    );
    assert_eq!(
        history_lines(&hist),
        vec![": c".to_string(), ": d".to_string()]
    );
}

#[test]
fn a_leading_space_keeps_a_command_out_of_the_history() {
    let dir = tempfile::tempdir().unwrap();
    let hist = dir.path().join("hist");
    repl(
        "echo one\n : secret\necho two\n",
        &[("HISTFILE", hist.to_str().unwrap())],
        dir.path(),
    );
    assert_eq!(
        history_lines(&hist),
        vec!["echo one".to_string(), "echo two".to_string()],
        "the leading-space convention must be honoured exactly once"
    );
}

#[test]
fn a_command_is_stored_once_not_twice() {
    let dir = tempfile::tempdir().unwrap();
    let hist = dir.path().join("hist");
    repl(
        "  echo hi  \n",
        &[("HISTFILE", hist.to_str().unwrap())],
        dir.path(),
    );
    // Leading whitespace also means "do not remember", so the file stays empty — what must not
    // happen is the line appearing twice.
    assert!(
        history_lines(&hist).is_empty(),
        "{:?}",
        history_lines(&hist)
    );

    let hist2 = dir.path().join("hist2");
    repl(
        "echo hi\n",
        &[("HISTFILE", hist2.to_str().unwrap())],
        dir.path(),
    );
    assert_eq!(history_lines(&hist2), vec!["echo hi".to_string()]);
}

#[test]
fn a_concurrent_sessions_entries_are_not_clobbered() {
    // The session appends to its own history file while it is running, standing in for a second
    // shell open at the same time. Rewriting the whole file on exit would lose that line.
    let dir = tempfile::tempdir().unwrap();
    let hist = dir.path().join("hist");
    let hist_str = hist.to_str().unwrap().to_string();

    repl(
        &format!("echo first\nprintf 'echo elsewhere\\n' >> {hist_str}\necho second\n"),
        &[("HISTFILE", &hist_str)],
        dir.path(),
    );

    let lines = history_lines(&hist);
    assert!(
        lines.contains(&"echo elsewhere".to_string()),
        "another session's entry was lost: {lines:?}"
    );
    assert!(lines.contains(&"echo second".to_string()), "{lines:?}");
}

#[test]
fn an_empty_histfile_disables_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let o = repl("echo hi\n", &[("HISTFILE", "")], dir.path());
    assert!(out(&o).contains("hi"));
    assert!(!dir.path().join(".oslo_history").exists());
}

#[test]
fn the_history_builtin_numbers_the_session() {
    let dir = tempfile::tempdir().unwrap();
    let o = repl(
        "echo one\necho two\nhistory\n",
        &[("HISTFILE", "")],
        dir.path(),
    );
    let text = out(&o);
    assert!(text.contains("    1  echo one"), "{text:?}");
    assert!(text.contains("    2  echo two"), "{text:?}");
    assert!(text.contains("    3  history"), "{text:?}");
}

#[test]
fn history_takes_a_count_and_minus_c() {
    let dir = tempfile::tempdir().unwrap();
    let o = repl(
        "echo one\necho two\nhistory 1\nhistory -c\nhistory\necho end\n",
        &[("HISTFILE", "")],
        dir.path(),
    );
    let text = out(&o);
    assert!(!text.contains("  1  echo one\n    2"), "{text:?}");
    // After `history -c` the only entry left is the `history` that reported it.
    assert!(text.contains("    1  history\n"), "{text:?}");
    assert!(text.contains("end"), "{text:?}");
}

#[test]
fn history_is_a_builtin_even_in_a_script() {
    let dir = tempfile::tempdir().unwrap();
    let o = run(&["-c", "type history; history; echo ok"], &[], dir.path());
    assert!(out(&o).contains("shell builtin"), "{:?}", out(&o));
    assert!(out(&o).contains("ok"));
    assert_eq!(o.status.code(), Some(0));
}

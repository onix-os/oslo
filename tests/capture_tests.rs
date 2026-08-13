//! `keep` and `copy --last` through the real binary: what a command printed, on the clipboard.

mod common;

use common::oslo_bin;
use std::process::Command;

/// Run one command line in an oslo with a capture directory of its own.
fn oslo(data: &std::path::Path, line: &str) -> (String, String) {
    let out = Command::new(oslo_bin())
        .args(["-c", line])
        .env("XDG_DATA_HOME", data)
        .stdin(std::process::Stdio::null())
        .output()
        .expect("spawn oslo");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

/// What `copy` sends: `OSC 52`, with the text in base64.
fn clipboard(text: &str) -> String {
    oslo_ui::marks::clipboard(text)
}

/// The whole point, end to end: run it, see it, copy it.
#[test]
fn what_was_kept_is_what_reaches_the_clipboard() {
    let data = tempfile::tempdir().expect("tempdir");
    let (out, _) = oslo(data.path(), "keep echo one two three; copy --last");

    assert!(
        out.starts_with("one two three\n"),
        "the output was not shown as it ran: {out:?}"
    );
    assert!(
        out.contains(&clipboard("one two three")),
        "the clipboard did not get it: {out:?}"
    );
}

/// **`keep` is transparent.** The command's own status is what the shell sees, or `keep make` would
/// be a way to lose a failure.
#[test]
fn the_commands_status_is_the_answer() {
    let data = tempfile::tempdir().expect("tempdir");
    let (out, _) = oslo(data.path(), "keep false; echo status=$?");
    assert!(out.contains("status=1"), "{out:?}");
}

/// stderr is the command's business by default, and `-e` is how it becomes part of the output.
#[test]
fn stderr_joins_only_when_it_is_asked_to() {
    let data = tempfile::tempdir().expect("tempdir");

    let (out, err) = oslo(
        data.path(),
        "keep sh -c 'echo out; echo err >&2'; copy --last",
    );
    assert!(out.contains(&clipboard("out")), "{out:?}");
    assert!(
        err.contains("err"),
        "stderr should still reach the terminal"
    );

    let (out, _) = oslo(
        data.path(),
        "keep -e sh -c 'echo out; echo err >&2'; copy --last",
    );
    assert!(out.contains(&clipboard("out\nerr")), "{out:?}");
}

/// A clipboard full of `\x1b[32m` is nobody's idea of the output.
#[test]
fn colours_do_not_reach_the_clipboard() {
    let data = tempfile::tempdir().expect("tempdir");
    let (out, _) = oslo(
        data.path(),
        "keep printf '\\033[32mgreen\\033[0m text'; copy --last",
    );
    assert!(out.contains(&clipboard("green text")), "{out:?}");
}

/// Nothing kept is not a failure of `copy`, and saying so is how anyone learns `keep` exists.
#[test]
fn a_session_that_kept_nothing_says_what_to_do() {
    let data = tempfile::tempdir().expect("tempdir");
    let (out, err) = oslo(data.path(), "copy --last; echo rc=$?");
    assert!(err.contains("keep"), "no way out was offered: {err:?}");
    assert!(out.contains("rc=1"), "{out:?}");
}

/// The last one is the last one: keeping again replaces what was kept.
#[test]
fn the_second_command_replaces_the_first() {
    let data = tempfile::tempdir().expect("tempdir");
    let (out, _) = oslo(
        data.path(),
        "keep echo first; keep echo second; copy --last",
    );
    assert!(out.contains(&clipboard("second")), "{out:?}");
    assert!(!out.contains(&clipboard("first")), "{out:?}");
}

/// **Two terminals are two shells.** "The last output" means a different thing in each, so a second
/// shell sharing the same store still starts with nothing of its own.
#[test]
fn another_shell_does_not_answer_with_yours() {
    let data = tempfile::tempdir().expect("tempdir");
    let (out, _) = oslo(data.path(), "keep echo mine; copy --last");
    assert!(out.contains(&clipboard("mine")), "{out:?}");

    let (out, err) = oslo(data.path(), "copy --last; echo rc=$?");
    assert!(
        !out.contains(&clipboard("mine")),
        "it answered with another shell's output: {out:?}"
    );
    assert!(err.contains("nothing kept"), "{err:?}");
}

/// Over the cap the tail is kept, and it says so rather than quietly shortening the answer.
#[test]
fn too_much_output_keeps_its_end_and_says_so() {
    let data = tempfile::tempdir().expect("tempdir");
    let (_, err) = oslo(
        data.path(),
        "keep sh -c \"head -c 1200000 /dev/zero | tr '\\0' x\" >/dev/null",
    );
    assert!(
        err.contains("kept the last"),
        "no word of the trim: {err:?}"
    );
}

/// A prefix in front of nothing is a usage error, not a command called "".
#[test]
fn a_prefix_with_nothing_after_it_is_refused() {
    let data = tempfile::tempdir().expect("tempdir");
    let (out, err) = oslo(data.path(), "keep; echo rc=$?");
    assert!(out.contains("rc=2"), "{out:?}");
    assert!(err.contains("usage"), "{err:?}");
}

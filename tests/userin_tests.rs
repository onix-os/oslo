//! `oslo userin` — the widgets, reached the way everything that is not oslo reaches them.
//!
//! The widgets themselves are tested where they live; what is tested here is the door: that the
//! subcommand exists, that it is the same list the builtin has, and that the three rules a script
//! depends on hold when it is a *program* answering rather than a builtin.

mod common;

use common::oslo_bin;
use std::process::Command;

fn userin(args: &[&str]) -> (String, String, i32) {
    let out = Command::new(oslo_bin())
        .arg("userin")
        .args(args)
        .stdin(std::process::Stdio::null())
        .output()
        .expect("spawn oslo");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
        out.status.code().unwrap_or(-1),
    )
}

/// **Every widget the builtin has**, or the door is only worth half of what it looks like.
#[test]
fn the_help_lists_the_widgets() {
    let (out, _, status) = userin(&["--help"]);
    assert_eq!(status, 0);
    for widget in [
        "input", "write", "confirm", "choose", "filter", "table", "file", "style", "format",
        "join", "pager", "log", "spin",
    ] {
        assert!(out.contains(widget), "{widget} is missing from the help");
    }
    // On stdout, because a `--help` on stderr cannot be paged or grepped.
    assert!(out.contains("usage: oslo userin"), "{out:?}");
}

/// **No terminal is 2, not 1.** A script has to be able to tell "there was nobody to ask" from
/// "they said no", and this is the only place that distinction can be made.
#[test]
fn nobody_to_ask_is_its_own_status() {
    let (out, _, status) = userin(&["choose", "alpha", "beta"]);
    assert_eq!(status, 2, "{out:?}");
    assert!(out.is_empty(), "an answer was printed with nobody there");
}

/// The half that needs no terminal works anywhere, including in a pipeline with no tty at all.
#[test]
fn what_needs_no_terminal_still_works() {
    let (out, _, status) = userin(&["style", "--border", "rounded", "done"]);
    assert_eq!(status, 0);
    assert!(out.contains("done"), "{out:?}");
}

/// A name that is not a widget is a usage error, and says which name it did not know.
#[test]
fn an_unknown_widget_is_refused() {
    let (_, err, status) = userin(&["nonesuch"]);
    assert_eq!(status, 2);
    assert!(err.contains("nonesuch"), "{err:?}");
    assert!(err.contains("usage: oslo userin"), "{err:?}");
}

/// With no widget at all there is nothing to do and nothing to guess.
#[test]
fn no_widget_is_a_usage_error() {
    let (out, err, status) = userin(&[]);
    assert_eq!(status, 2, "{out:?}");
    assert!(err.contains("usage: oslo userin"), "{err:?}");
}

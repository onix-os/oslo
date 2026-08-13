//! `$OSLO_NESTED`: how many oslo shells deep a shell is.
//!
//! The question a nested *interactive* shell asks needs a terminal to draw on, so it is not here;
//! what is here is the count it is asked about, and the cases where nothing must be asked at all.

mod common;

use common::oslo_bin;
use std::process::Command;

/// The controlling terminal, the way the shell reads it — `None` where there is none, which is the
/// usual case under a test runner.
fn terminal() -> Option<String> {
    use std::os::unix::fs::MetadataExt;
    let tty = std::fs::File::open("/dev/tty").ok()?;
    Some(tty.metadata().ok()?.rdev().to_string())
}

/// Run `line` in an oslo, with `nested` as the inherited `$OSLO_NESTED` (or none).
///
/// The terminal is published alongside it, because that is what the shell that set the count would
/// have done — a count with no terminal beside it is one from another screen, and is ignored.
fn oslo(nested: Option<&str>, line: &str) -> String {
    let mut command = Command::new(oslo_bin());
    command
        .args(["-c", line])
        .env_remove("OSLO_NESTED")
        .env_remove("OSLO_NESTED_TTY")
        .stdin(std::process::Stdio::null());
    if let Some(value) = nested {
        command.env("OSLO_NESTED", value);
        if let Some(tty) = terminal() {
            command.env("OSLO_NESTED_TTY", tty);
        }
    }
    let out = command.output().expect("spawn oslo");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// A fresh terminal has nothing above it, and says so rather than saying nothing.
#[test]
fn the_first_shell_is_the_zeroth() {
    assert_eq!(oslo(None, "echo $OSLO_NESTED"), "0");
}

/// **The count is what makes it usable in a prompt**: one deep is 1, two deep is 2.
#[test]
fn each_shell_stands_one_below_the_one_that_started_it() {
    assert_eq!(oslo(Some("0"), "echo $OSLO_NESTED"), "1");
    assert_eq!(oslo(Some("1"), "echo $OSLO_NESTED"), "2");
    assert_eq!(oslo(Some("7"), "echo $OSLO_NESTED"), "8");
}

/// The real thing, through two processes rather than through a variable set by hand.
#[test]
fn a_shell_inside_a_shell_counts_itself() {
    let inner = format!("{} -c 'echo $OSLO_NESTED'", oslo_bin().display());
    assert_eq!(oslo(None, &inner), "1");
}

/// Somebody else's `OSLO_NESTED=deep` is not a depth, and a shell will not invent one from it.
#[test]
fn a_value_that_is_not_a_number_is_not_a_shell() {
    assert_eq!(oslo(Some("deep"), "echo $OSLO_NESTED"), "0");
    assert_eq!(oslo(Some(""), "echo $OSLO_NESTED"), "0");
}

/// **A count from another screen is somebody else's.** `tmux`, `hexe` and `ssh` all inherit the
/// variable from the shell that started them and run their own shell on a pty of its own; treating
/// that as nesting asked a fresh pane whether it meant to nest, and left `⧉1` in its prompt for
/// ever. Here the terminal is deliberately not published beside the count.
#[test]
fn a_count_from_another_terminal_is_ignored() {
    let out = Command::new(oslo_bin())
        .args(["-c", "echo $OSLO_NESTED"])
        .env("OSLO_NESTED", "3")
        .env("OSLO_NESTED_TTY", "somewhere-else")
        .stdin(std::process::Stdio::null())
        .output()
        .expect("spawn oslo");
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "0");
}

/// **A shell with no terminal is never asked anything.** `command | oslo -i` sets `interactive`
/// and has no person on the other end of stdin; a question there would be answered by the script's
/// first line, and every test suite that pipes into an oslo would hang or exit.
#[test]
fn a_shell_with_no_terminal_just_runs() {
    use std::io::Write;
    let mut child = Command::new(oslo_bin())
        .arg("-i")
        .env("OSLO_NESTED", "3")
        .envs(terminal().map(|tty| ("OSLO_NESTED_TTY", tty)))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn oslo");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(b"echo ALIVE=$OSLO_NESTED\n")
        .expect("write");
    let out = child.wait_with_output().expect("wait");
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("ALIVE=4"),
        "a piped interactive shell did not run: {:?}",
        String::from_utf8_lossy(&out.stdout)
    );
}

//! `$OSLO_NESTED`: how many oslo shells deep a shell is.
//!
//! The question a nested *interactive* shell asks needs a terminal to draw on, so it is not here;
//! what is here is the count it is asked about, and the cases where nothing must be asked at all.

mod common;

use common::oslo_bin;
use std::process::Command;

/// Run `line` in an oslo, with `nested` as the inherited `$OSLO_NESTED` (or none).
fn oslo(nested: Option<&str>, line: &str) -> String {
    let mut command = Command::new(oslo_bin());
    command
        .args(["-c", line])
        .env_remove("OSLO_NESTED")
        .stdin(std::process::Stdio::null());
    if let Some(value) = nested {
        command.env("OSLO_NESTED", value);
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

/// **A shell with no terminal is never asked anything.** `command | oslo -i` sets `interactive`
/// and has no person on the other end of stdin; a question there would be answered by the script's
/// first line, and every test suite that pipes into an oslo would hang or exit.
#[test]
fn a_shell_with_no_terminal_just_runs() {
    use std::io::Write;
    let mut child = Command::new(oslo_bin())
        .arg("-i")
        .env("OSLO_NESTED", "3")
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

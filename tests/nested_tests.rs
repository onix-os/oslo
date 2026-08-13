//! `$OSLO_NESTED`: how many oslo shells a shell is standing inside.
//!
//! The question a nested shell asks needs a terminal to draw on, so it is not here; what is here is
//! the count it is asked about, and the cases where nothing must be asked at all.
//!
//! Every child gets **no terminal of any kind**, so what it makes of the count is the same whether
//! this suite was started from a terminal or from CI. A shell with no terminal and a count with no
//! terminal beside it are on the same screen — which is to say neither is on one, and there is
//! nobody there for the difference to matter to.

mod common;

use common::oslo_bin;
use std::io::Write;
use std::process::Command;

/// A file holding one line, to be fed to a shell as its standard input.
///
/// The alternative is `printf … | oslo -i` inside a `-c` inside another shell, where the `$` in the
/// line has to survive two rounds of quoting — and does not: the outer shell expands it, and the
/// test then measures its own quoting rather than the shell's counting.
fn typed(line: &str) -> tempfile::NamedTempFile {
    let mut file = tempfile::NamedTempFile::new().expect("temporary file");
    writeln!(file, "{line}").expect("write");
    file.flush().expect("flush");
    file
}

fn bin() -> String {
    oslo_bin().display().to_string()
}

/// Run `line` in a `-c` shell — the kind that is plumbing and takes no level.
fn wrapper(nested: Option<&str>, line: &str) -> String {
    let mut command = Command::new(oslo_bin());
    command
        .args(["-c", line])
        .env_remove("OSLO_NESTED")
        .env_remove("OSLO_NESTED_TTY")
        .env_remove("OSLO_NESTED_PID")
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    if let Some(value) = nested {
        command.env("OSLO_NESTED", value);
        // This process is about to be the child's parent, which is what a shell publishing a count
        // is: something the next shell is genuinely inside.
        command.env("OSLO_NESTED_PID", std::process::id().to_string());
    }
    let out = command.output().expect("spawn oslo");
    String::from_utf8_lossy(&out.stdout).to_string()
}

/// Type `line` at an oslo that is interactive — the only kind that takes a level.
///
/// `-i` with stdin on a pipe is interactive without being on a terminal, which is what lets the
/// counting be tested at all without a pty.
fn at_a_prompt(nested: Option<&str>, line: &str) -> String {
    let mut command = Command::new(oslo_bin());
    command
        .arg("-i")
        .env_remove("OSLO_NESTED")
        .env_remove("OSLO_NESTED_TTY")
        .env_remove("OSLO_NESTED_PID")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    if let Some(value) = nested {
        command.env("OSLO_NESTED", value);
        command.env("OSLO_NESTED_PID", std::process::id().to_string());
    }
    let mut child = command.spawn().expect("spawn oslo");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(format!("{line}\n").as_bytes())
        .expect("write");
    let out = child.wait_with_output().expect("wait");
    String::from_utf8_lossy(&out.stdout).to_string()
}

/// A fresh terminal has nothing above it, and says so rather than saying nothing.
#[test]
fn the_first_shell_is_the_zeroth() {
    assert!(at_a_prompt(None, "echo D=$OSLO_NESTED").contains("D=0"));
}

/// **The count is what makes it usable in a prompt**: one deep is 1, two deep is 2.
#[test]
fn each_shell_stands_one_below_the_one_that_started_it() {
    for (above, mine) in [("0", "D=1"), ("1", "D=2"), ("7", "D=8")] {
        let out = at_a_prompt(Some(above), "echo D=$OSLO_NESTED");
        assert!(out.contains(mine), "under {above}: {out:?}");
    }
}

/// The real thing, through two processes rather than through a variable set by hand.
#[test]
fn a_shell_inside_a_shell_counts_itself() {
    let script = typed("echo IN=$OSLO_NESTED");
    let out = at_a_prompt(None, &format!("{} -i < {}", bin(), script.path().display()));
    assert!(out.contains("IN=1"), "{out:?}");
}

/// **A `-c` shell is not a level, and this is the `ssh` bug.** sshd on the far side runs
/// `oslo -c waypipe … server` and the login shell comes out from under it — the same terminal, a
/// live ancestor, and nothing anybody could `exit` back into. Every login was asked whether it
/// meant to nest, and no amount of checking the terminal or the ancestry could have caught it:
/// both were true. What was wrong was calling a wrapper a shell you are inside.
#[test]
fn a_wrapper_shell_does_not_make_the_next_one_nested() {
    let script = typed("echo IN=$OSLO_NESTED");
    let out = wrapper(None, &format!("{} -i < {}", bin(), script.path().display()));
    assert!(
        out.contains("IN=0"),
        "the login shell was told it was nested: {out:?}"
    );
}

/// What a wrapper passes on is what it was handed: the shell above *it*, unchanged.
#[test]
fn a_wrapper_passes_the_stack_through() {
    let script = typed("echo IN=$OSLO_NESTED");
    let out = at_a_prompt(
        None,
        &format!("{} -c '{} -i < {}'", bin(), bin(), script.path().display()),
    );
    assert!(
        out.contains("IN=1"),
        "a shell under a wrapper under a prompt is one deep, not two: {out:?}"
    );
}

/// Somebody else's `OSLO_NESTED=deep` is not a depth, and a shell will not invent one from it.
#[test]
fn a_value_that_is_not_a_number_is_not_a_shell() {
    assert!(at_a_prompt(Some("deep"), "echo D=$OSLO_NESTED").contains("D=0"));
    assert!(at_a_prompt(Some(""), "echo D=$OSLO_NESTED").contains("D=0"));
}

/// **A count from another screen is somebody else's.** `tmux`, `hexe` and `ssh` all inherit the
/// variable from the shell that started them and run their own shell on a pty of its own; treating
/// that as nesting asked every fresh pane whether it meant to nest, and left `⧉1` in its prompt for
/// ever.
#[test]
fn a_count_from_another_terminal_is_ignored() {
    let mut command = Command::new(oslo_bin());
    command
        .arg("-i")
        .env("OSLO_NESTED", "3")
        .env("OSLO_NESTED_TTY", "somewhere-else")
        .env("OSLO_NESTED_PID", std::process::id().to_string())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    let mut child = command.spawn().expect("spawn oslo");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(b"echo D=$OSLO_NESTED\n")
        .expect("write");
    let out = child.wait_with_output().expect("wait");
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("D=0"),
        "{:?}",
        String::from_utf8_lossy(&out.stdout)
    );
}

/// **A count whose shell is gone is nobody's.** A `tmux` or `hexe` server keeps the environment it
/// was started with and hands it to every pane for as long as it runs, so a pane opened today can
/// inherit a count from a shell that exited last week — the case where the terminal check cannot
/// help, because the pane may well be on the same screen the count was set on.
#[test]
fn a_count_from_a_shell_that_is_gone_is_ignored() {
    let mut command = Command::new(oslo_bin());
    command
        .arg("-i")
        .env("OSLO_NESTED", "2")
        .env_remove("OSLO_NESTED_TTY")
        // Everything else agrees; only the shell that said so is not there.
        .env("OSLO_NESTED_PID", "4294967294")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    let mut child = command.spawn().expect("spawn oslo");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(b"echo D=$OSLO_NESTED\n")
        .expect("write");
    let out = child.wait_with_output().expect("wait");
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("D=0"),
        "{:?}",
        String::from_utf8_lossy(&out.stdout)
    );
}

/// **A shell with no terminal is never asked anything.** `command | oslo -i` sets `interactive`
/// and has no person on the other end of stdin; a question there would be answered by the script's
/// first line, and every test suite that pipes into an oslo would hang or exit.
#[test]
fn a_shell_with_no_terminal_just_runs() {
    let out = at_a_prompt(Some("3"), "echo ALIVE=$OSLO_NESTED");
    assert!(out.contains("ALIVE=4"), "{out:?}");
}

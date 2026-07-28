//! The shell's own command line (PLAN R1.10).
//!
//! Every one of these used to start a REPL: `main` recognised `-c`, `--lua-script` and a bare
//! script path, and treated everything else — `--version`, `--help`, `-Z`, even a bare `-c` with
//! no argument — as "no arguments", which read the caller's stdin and exited 0. A shell that
//! swallows its input on a typo is worse than one that refuses to start.

mod common;

use common::rush_bin;
use std::io::Write;
use std::process::{Command, Output, Stdio};

fn rush(args: &[&str]) -> Output {
    Command::new(rush_bin())
        .args(args)
        .stdin(Stdio::null())
        .output()
        .expect("spawn rush")
}

fn stdout_of(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr_of(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn status_of(out: &Output) -> i32 {
    out.status.code().unwrap_or(-1)
}

#[test]
fn version_prints_the_crate_version_and_exits_zero() {
    let out = rush(&["--version"]);
    assert_eq!(status_of(&out), 0);
    let text = stdout_of(&out);
    assert!(text.contains("rush"), "{text:?}");
    assert!(
        text.contains(env!("CARGO_PKG_VERSION")),
        "the version must come from Cargo, not a hardcoded string: {text:?}"
    );
}

#[test]
fn help_prints_usage_on_stdout_and_exits_zero() {
    let out = rush(&["--help"]);
    assert_eq!(status_of(&out), 0);
    let text = stdout_of(&out);
    assert!(text.contains("usage: rush"), "{text:?}");
    assert!(text.contains("-c COMMAND"), "{text:?}");
    assert!(stderr_of(&out).is_empty());
}

#[test]
fn an_unknown_option_prints_usage_on_stderr_and_exits_two() {
    let out = rush(&["-Z"]);
    assert_eq!(status_of(&out), 2);
    let err = stderr_of(&out);
    assert!(err.contains("-Z"), "{err:?}");
    assert!(err.contains("usage: rush"), "{err:?}");
    assert!(
        stdout_of(&out).is_empty(),
        "usage errors do not go to stdout"
    );
}

#[test]
fn an_unknown_long_option_exits_two() {
    let out = rush(&["--nonesuch"]);
    assert_eq!(status_of(&out), 2);
    assert!(stderr_of(&out).contains("--nonesuch"));
}

#[test]
fn dash_c_without_an_argument_exits_two() {
    let out = rush(&["-c"]);
    assert_eq!(status_of(&out), 2);
    assert!(
        stderr_of(&out).contains("requires an argument"),
        "{:?}",
        stderr_of(&out)
    );
}

#[test]
fn dash_c_still_runs_a_command() {
    let out = rush(&["-c", "echo hi"]);
    assert_eq!(status_of(&out), 0);
    assert_eq!(stdout_of(&out), "hi\n");
}

#[test]
fn dash_c_takes_name_and_positional_operands() {
    let out = rush(&["-c", "echo $0 $# $1 $2", "myname", "one", "two"]);
    assert_eq!(stdout_of(&out).trim_end(), "myname 2 one two");
}

#[test]
fn a_script_operand_gets_its_own_positionals() {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("s.sh");
    std::fs::write(&script, "echo $# $1 $2\n").unwrap();

    let out = rush(&[script.to_str().unwrap(), "alpha", "beta"]);
    assert_eq!(stdout_of(&out).trim_end(), "2 alpha beta");
    assert_eq!(status_of(&out), 0);
}

#[test]
fn a_missing_script_exits_one_hundred_and_twenty_seven() {
    let out = rush(&["/nonexistent/script/xyz.sh"]);
    assert_eq!(status_of(&out), 127);
    assert!(!stderr_of(&out).is_empty());
}

#[test]
fn double_dash_ends_option_parsing() {
    let out = rush(&["--", "/nonexistent/script/xyz.sh"]);
    assert_eq!(
        status_of(&out),
        127,
        "the operand after -- is a script name, not an option"
    );
}

/// A non-tty stdin means a script to run, not a session to start. With no input there is nothing
/// to run, so the shell exits 0 immediately instead of blocking on a prompt.
#[test]
fn stdin_from_dev_null_exits_zero_without_a_banner() {
    let out = rush(&[]);
    assert_eq!(status_of(&out), 0);
    assert!(
        !stdout_of(&out).contains("POSIX Compatible Shell"),
        "the banner belongs to interactive sessions only: {:?}",
        stdout_of(&out)
    );
}

#[test]
fn a_program_piped_into_stdin_is_executed() {
    for args in [vec![], vec!["-s"]] {
        let mut child = Command::new(rush_bin())
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn rush");
        child
            .stdin
            .take()
            .expect("stdin")
            .write_all(b"x=7\necho piped $x\n")
            .expect("write script");
        let out = child.wait_with_output().expect("wait");
        assert_eq!(stdout_of(&out), "piped 7\n", "args: {args:?}");
        assert_eq!(status_of(&out), 0);
    }
}

#[test]
fn a_syntax_error_from_stdin_exits_two() {
    let mut child = Command::new(rush_bin())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn rush");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(b"echo a; )\n")
        .expect("write script");
    let out = child.wait_with_output().expect("wait");
    assert_eq!(status_of(&out), 2);
}

#[test]
fn recorded_set_options_do_not_prevent_a_script_from_running() {
    // -e and -x are accepted and recorded; Round 6 gives them meaning. Until then they must at
    // least not be mistaken for a script name or an unknown option.
    let out = rush(&["-ex", "-c", "echo ran"]);
    assert_eq!(status_of(&out), 0, "stderr: {}", stderr_of(&out));
    assert_eq!(stdout_of(&out), "ran\n");
}

//! Names and values the process environment cannot hold (R1.7).
//!
//! `std::env::set_var` panics on an empty name, a `=` inside a name, or a NUL anywhere, and a
//! panic in the interpreter loop takes the whole session with it — `export "=1"` used to kill an
//! interactive shell outright. Every test here therefore checks two things: the right status,
//! and that the shell is *still running* afterwards. Spawning the real binary is what makes the
//! second check meaningful; an in-process test would abort the test runner instead.

mod common;

use common::{run, run_in};

/// Proof of life: if the shell died on the previous command this never reaches stdout.
const SENTINEL: &str = "STILL_ALIVE";

fn assert_alive(script: &str) -> common::Run {
    let r = run(&format!("{}; echo {}", script, SENTINEL));
    assert!(
        r.out().ends_with(SENTINEL),
        "shell did not survive `{}`\nstdout: {:?}\nstderr: {}\nstatus: {}",
        script,
        r.stdout,
        r.stderr,
        r.status
    );
    r
}

#[test]
fn export_of_an_empty_name_is_an_error_not_a_crash() {
    let r = assert_alive(r#"export "=1""#);
    assert!(
        r.stderr.contains("not a valid identifier"),
        "stderr: {}",
        r.stderr
    );
    // The status of `export` itself, not of the sentinel `echo` after it.
    assert_eq!(run(r#"export "=1""#).status, 1);
}

#[test]
fn export_rejects_every_shape_of_invalid_name() {
    for bad in [
        r#"export "=1""#,
        "export '1abc=x'",
        r#"export "a b=1""#,
        "export 'a-b'",
    ] {
        let r = assert_alive(bad);
        assert!(
            r.stderr.contains("not a valid identifier"),
            "{bad}: {}",
            r.stderr
        );
        assert_eq!(run(bad).status, 1, "{bad} should exit 1");
    }
}

#[test]
fn local_of_an_invalid_name_is_an_error_not_a_crash() {
    let r = assert_alive(r#"f() { local "=1"; echo "inner=$?"; }; f"#);
    assert!(
        r.stderr.contains("not a valid identifier"),
        "stderr: {}",
        r.stderr
    );
    assert!(r.out().contains("inner=1"), "stdout: {:?}", r.stdout);
}

#[test]
fn readonly_of_an_invalid_name_is_an_error_not_a_crash() {
    let r = assert_alive(r#"readonly "=1"; echo "st=$?""#);
    assert!(
        r.stderr.contains("not a valid identifier"),
        "stderr: {}",
        r.stderr
    );
    assert!(r.out().contains("st=1"), "stdout: {:?}", r.stdout);
}

/// `unset` reaches `remove_var`, which panics on the same names `set_var` does.
#[test]
fn unset_of_an_impossible_name_is_a_no_op() {
    let r = assert_alive(r#"unset "=1""#);
    assert_eq!(r.status, 0, "stderr: {}", r.stderr);
}

/// A NUL can enter the shell from a file even though it can never enter argv.
#[test]
fn a_nul_bearing_value_never_reaches_the_environment() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("nul.bin"), b"a\0b\n").expect("write");

    let r = run_in(
        dir.path(),
        &format!("read X < nul.bin; export X; printenv X; echo {}", SENTINEL),
    );
    assert!(
        r.out().ends_with(SENTINEL),
        "shell died on a NUL-bearing value\nstdout: {:?}\nstderr: {}",
        r.stdout,
        r.stderr
    );
    assert!(
        !r.stdout.contains('\0'),
        "a NUL reached the child environment: {:?}",
        r.stdout
    );
}

/// The whole point of the validation is that ordinary exports keep working, including the trip
/// through `environ` into a child process.
#[test]
fn valid_exports_still_reach_child_processes() {
    let r = run("export FOO=bar; printenv FOO");
    assert_eq!(r.out(), "bar", "stderr: {}", r.stderr);
    assert_eq!(r.status, 0);

    // Underscore-led and digit-bearing names are valid; only a *leading* digit is not.
    let r = run("export _x9=ok; printenv _x9");
    assert_eq!(r.out(), "ok", "stderr: {}", r.stderr);
}

/// A rejected name must not abandon the arguments after it.
#[test]
fn export_continues_past_a_rejected_name() {
    let r = run(r#"export "=1" GOOD=yes; echo "st=$?"; printenv GOOD"#);
    assert!(r.out().contains("st=1"), "stdout: {:?}", r.stdout);
    assert!(r.out().contains("yes"), "stdout: {:?}", r.stdout);
}

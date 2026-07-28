//! argv handed to `execv` must survive hostile bytes (PLAN R1.6).
//!
//! The exec path used to be two `unwrap`s away from death: `CString::new(word).unwrap()` turned a
//! single NUL byte anywhere in an argument into a Rust panic and exit 101, and
//! `path.to_str().unwrap()` did the same for a resolved binary path that was not UTF-8. Both are
//! reachable from ordinary input — `read` can put a NUL in a variable, a PATH entry is an
//! arbitrary byte string — so both killed the shell instead of failing the command.
//!
//! These assert the *shell survives*, deliberately not which earlier layer neutralises the NUL:
//! command substitution strips it, `read` and `export` reject it, and the exec path drops whatever
//! still gets through. Any of those may tighten without invalidating the invariant tested here.
//! The dropping itself is unit-tested in `src/exec/simple.rs`, since with those layers in place no
//! script can currently hand a NUL to `execv`.

mod common;

use common::rush_bin;
use std::fs;
use std::path::Path;
use std::process::{Command, Output, Stdio};

struct Run {
    stdout: String,
    stderr: String,
    status: i32,
}

fn run_in(dir: &Path, path_var: Option<&str>, script: &str) -> Run {
    let mut cmd = Command::new(rush_bin());
    cmd.arg("-c").arg(script).current_dir(dir);
    if let Some(p) = path_var {
        cmd.env("PATH", p);
    }
    let out: Output = cmd.stdin(Stdio::null()).output().expect("spawn rush");
    Run {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        status: out.status.code().unwrap_or(-1),
    }
}

/// No panic, no 101, and the script kept running past the offending command.
fn assert_survived(r: &Run, what: &str) {
    assert!(
        !r.stderr.contains("panicked"),
        "{what}: shell panicked: {}",
        r.stderr
    );
    assert_ne!(r.status, 101, "{what}: shell aborted: {}", r.stderr);
    assert!(
        r.stdout.contains("STILL_ALIVE"),
        "{what}: shell died mid-script: stdout {:?} stderr {}",
        r.stdout,
        r.stderr
    );
}

/// `read` is the shortest route from a file's bytes into a word: a NUL in the file used to reach
/// `CString::new(..).unwrap()` and take the shell down with exit 101.
#[test]
fn nul_from_read_does_not_kill_the_shell() {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(dir.path().join("f.txt"), b"a\0b\n").expect("write");

    let r = run_in(
        dir.path(),
        None,
        "read x < f.txt; /bin/echo \"[$x]\"; echo STILL_ALIVE",
    );
    assert_survived(&r, "NUL from read");
}

/// The PLAN repro. Command substitution drops the NUL the way bash does, so this one has an exact
/// expected output.
#[test]
fn nul_from_command_substitution_prints_stripped_value() {
    let dir = tempfile::tempdir().expect("tempdir");
    let r = run_in(
        dir.path(),
        None,
        "x=$(printf 'a\\0b'); /bin/echo \"[$x]\"; echo STILL_ALIVE",
    );
    assert_survived(&r, "NUL from substitution");
    assert!(
        r.stdout.contains("[ab]"),
        "expected [ab], got {:?} (stderr {})",
        r.stdout,
        r.stderr
    );
    assert_eq!(r.status, 0);
}

/// A NUL in the *command name* takes the argv[0] path instead of an argument.
#[test]
fn nul_in_command_name_does_not_kill_the_shell() {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(dir.path().join("f.txt"), b"no\0such\0cmd\n").expect("write");

    let r = run_in(dir.path(), None, "read c < f.txt; $c; echo STILL_ALIVE");
    assert_survived(&r, "NUL in command name");
}

/// A NUL reaching a *pipeline* member exercises the same conversion in a forked child.
#[test]
fn nul_in_pipeline_argument_does_not_kill_the_shell() {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(dir.path().join("f.txt"), b"a\0b\n").expect("write");

    let r = run_in(
        dir.path(),
        None,
        "read x < f.txt; /bin/echo \"$x\" | /bin/cat; echo STILL_ALIVE",
    );
    assert_survived(&r, "NUL in pipeline");
}

/// A PATH holding entries that are not directories, plus an empty entry, must not stop the lookup
/// or the exec that follows.
#[test]
fn garbage_path_entries_are_skipped() {
    let dir = tempfile::tempdir().expect("tempdir");
    let r = run_in(
        dir.path(),
        Some("/nonexistent/\u{1}x::/etc/hostname:/usr/bin:/bin"),
        "/bin/echo hi; echo STILL_ALIVE",
    );
    assert_survived(&r, "garbage PATH");
    assert!(r.stdout.contains("hi"), "stdout {:?}", r.stdout);
}

/// A PATH entry that resolves the command through a directory with a non-ASCII name: the resolved
/// path is what `path.to_str().unwrap()` used to choke on for non-UTF-8 bytes.
#[test]
fn unusual_path_entry_still_execs() {
    let dir = tempfile::tempdir().expect("tempdir");
    let bin_dir = dir.path().join("bü n");
    fs::create_dir(&bin_dir).expect("mkdir");
    fs::copy("/bin/echo", bin_dir.join("rushtestecho")).expect("copy echo");

    let path = format!("{}:/usr/bin:/bin", bin_dir.display());
    let r = run_in(dir.path(), Some(&path), "rushtestecho ok; echo STILL_ALIVE");
    assert_survived(&r, "non-ASCII PATH entry");
    assert!(r.stdout.contains("ok"), "stdout {:?}", r.stdout);
}

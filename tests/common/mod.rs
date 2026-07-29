//! Shared harness for the end-to-end suites.
//!
//! Compiled into each test binary separately, so any helper a given suite does not use looks
//! dead to that build — hence the allow.
#![allow(dead_code)]
//!
//! These exist because the in-process tests in `posix_shell_tests.rs` cannot catch a whole class
//! of defect. They build an AST and inspect `Environment` afterwards, so anything that goes wrong
//! *between* the AST and the observable behaviour of the shell — a redirection dropped during AST
//! conversion, an exit status never propagated to `main`, an alias mangled on its way to `execv`
//! — leaves the environment looking perfectly correct.
//!
//! Every test here therefore goes through the same path a user does: argv in, stdout and exit
//! code out.
//!
//! The second reason to reach for this harness is isolation. Process-global state — `environ`,
//! the working directory, the umask — belongs to the *process*, not to the test, and libtest runs
//! tests as threads of one process. A test that changes any of those in process is not merely
//! flaky against its neighbours; `std::env::set_var` racing another thread's `getenv` is
//! undefined behaviour. Spawning the binary gives each such test its own `environ`, its own cwd
//! and its own umask by construction, with no cooperative locking to remember to take.

use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

/// Path to the binary under test, as laid down next to the integration test executable.
pub fn rush_bin() -> PathBuf {
    // target/debug/deps/<test>-<hash> -> target/debug/rush
    let mut p = std::env::current_exe().expect("test executable path");
    p.pop();
    if p.ends_with("deps") {
        p.pop();
    }
    p.push("rush");
    assert!(
        p.exists(),
        "rush binary not found at {} — run `cargo build` first",
        p.display()
    );
    p
}

pub struct Run {
    pub stdout: String,
    pub stderr: String,
    pub status: i32,
}

impl Run {
    pub fn out(&self) -> &str {
        self.stdout.trim_end()
    }

    pub fn err(&self) -> &str {
        self.stderr.trim_end()
    }

    /// stdout split into lines with the trailing blank removed, for scripts that print a
    /// sequence of observations (a `pwd` before and after a `cd`, say).
    pub fn lines(&self) -> Vec<&str> {
        self.out().lines().collect()
    }
}

/// Run `script` through `rush -c` in a scratch directory, with stdin closed.
///
/// stdin is `/dev/null` deliberately: a dropped input redirection makes the command read the
/// real stdin instead of the file, which without this would hang the test run rather than fail it.
pub fn run_in(dir: &std::path::Path, script: &str) -> Run {
    let output: Output = Command::new(rush_bin())
        .arg("-c")
        .arg(script)
        .current_dir(dir)
        .stdin(Stdio::null())
        .output()
        .expect("spawn rush");

    Run {
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        status: output.status.code().unwrap_or(-1),
    }
}

/// Run in a fresh temporary directory.
pub fn run(script: &str) -> Run {
    let dir = tempfile::tempdir().expect("tempdir");
    run_in(dir.path(), script)
}

pub fn assert_out(script: &str, expected: &str) {
    let r = run(script);
    assert_eq!(
        r.out(),
        expected,
        "script: {}\nstderr: {}",
        script,
        r.stderr
    );
}

/// As [`assert_out`], but in a directory the caller controls — needed whenever the script's
/// meaning depends on where it runs (`pwd`, `pushd`, a relative command word).
pub fn assert_out_in(dir: &std::path::Path, script: &str, expected: &str) {
    let r = run_in(dir, script);
    assert_eq!(
        r.out(),
        expected,
        "script: {}\ncwd: {}\nstderr: {}",
        script,
        dir.display(),
        r.stderr
    );
}

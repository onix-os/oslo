//! Any `nix` command that speaks JSON, run as a process and answered as text.
//!
//! The generic half of the `nix` feature. [`super::apply`] asks nix one fixed question; this asks
//! whatever it is given, which is what `oslo.nix.run` in a config needs.
//!
//! # Why this is generic rather than a function per command
//!
//! On nix 2.34.6, twenty-three subcommands advertise `--json`, and nix says in its own `--help`
//! that the interface is subject to change. Two of those advertisements are already false:
//! `nix registry list --help` documents `--json` and `nix registry list --json` answers
//! `error: unrecognised flag '--json'`. A wrapper per command would have shipped one that cannot
//! work; this reports it as a message and lets the caller decide.
//!
//! # Argv, never a shell
//!
//! [`super::command`] builds a *string* for `eval_command_substitution`, because that is what the
//! one fixed command needed. Arguments arriving from a config are a different problem — quoting
//! them back into a string to have a shell take them apart again is where the injections live — so
//! this uses `Command` and passes the list through untouched.

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Long enough that no honest query hits it, short enough to be a ceiling.
///
/// **Set by the worst measurement, not a guess.** `nix search nixpkgs ripgrep --json` took 46
/// seconds here on a cold evaluation cache. Something that can hold a prompt for the better part of
/// a minute needs a limit it cannot exceed without being asked.
pub const TIMEOUT: Duration = Duration::from_secs(60);

/// Flags every invocation carries, whatever the caller asked for.
///
/// The same pair [`super::command`] passes, and for the same reason: these subcommands are all
/// behind `nix-command`, and a user who has not enabled it in `nix.conf` would otherwise get a
/// lecture about experimental features instead of an answer.
const ENABLE: [&str; 2] = ["--extra-experimental-features", "nix-command flakes"];

/// Run `nix` with `argv` and answer what it wrote to stdout.
///
/// `--json` is appended unless `argv` already carries it, so both `{"flake", "metadata"}` and
/// `{"flake", "metadata", "--json"}` mean the same thing and neither produces a duplicate flag.
///
/// A non-zero exit is an `Err` holding stderr rather than a panic or an empty document: a flake
/// that does not exist and a flag that is not recognised are both conditions a config branches on.
pub fn run(argv: &[String], timeout: Duration) -> Result<String, String> {
    run_program("nix".as_ref(), argv, timeout)
}

/// [`run`], against a named program.
///
/// **The seam the tests use.** CI has no nix, so they run a script that impersonates it — and doing
/// that by rewriting `$PATH` would mean a process-global mutation (`unsafe` since edition 2024)
/// serialised across every test that spawns anything. A parameter costs one line and none of that.
fn run_program(
    program: &std::ffi::OsStr,
    argv: &[String],
    timeout: Duration,
) -> Result<String, String> {
    let mut command = Command::new(program);
    command.args(ENABLE).args(argv);
    if !argv.iter().any(|a| a == "--json") {
        command.arg("--json");
    }

    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        // Captured rather than inherited: it is the error message, and the caller is a script that
        // wants to read it, not a person watching a terminal.
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => "nix is not installed".to_string(),
            _ => e.to_string(),
        })?;

    // **Drained on their own threads, before anything waits.** `nix config show --json` is 76 KB
    // and a pipe buffer is 64 KB: a child that fills it blocks in `write` while the parent blocks
    // in `try_wait`, and neither ever moves. Reading both pipes concurrently is the only shape that
    // cannot deadlock, and the largest document is the one most worth having.
    let out = child.stdout.take().map(drain);
    let err = child.stderr.take().map(drain);

    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(2)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                // **Returned without joining the readers**, which is the whole of the deadline
                // being a deadline. Killing nix does not close the pipe if nix left a child holding
                // it, so a join here waits for the *grandchild* — a 150 ms limit spent 30 seconds
                // in the test that found this. The output was already decided to be worthless; the
                // threads end when the pipe finally closes and nothing is listening.
                // `Duration`'s own formatting, because `as_secs` truncates and a sub-second limit
                // reported itself as "timed out after 0s".
                return Err(format!(
                    "`nix {}` timed out after {timeout:?}",
                    argv.join(" ")
                ));
            }
            // `ECHILD` — something reaped the child first, which a shell with `SIGCHLD` set to
            // `SIG_IGN` does routinely. It means finished, not failed, so read what it wrote. The
            // same case bit `lua::api::external::run`, where a test found it.
            Err(_) => break None,
        }
    };

    let stdout = out.map(join).unwrap_or_default();
    let stderr = err.map(join).unwrap_or_default();

    match status {
        Some(status) if status.success() => Ok(stdout),
        Some(_) if !stderr.trim().is_empty() => Err(stderr.trim().to_string()),
        Some(status) => Err(format!("nix exited {}", status.code().unwrap_or(-1))),
        // The `ECHILD` path, which is the only one left: no status to test, and its output is the
        // only evidence of how it went. Refusing it here would turn every run under a `SIG_IGN`
        // `SIGCHLD` into a failure.
        None => Ok(stdout),
    }
}

/// Read a pipe to the end on its own thread.
fn drain(mut pipe: impl Read + Send + 'static) -> std::thread::JoinHandle<String> {
    std::thread::spawn(move || {
        let mut text = String::new();
        let _ = pipe.read_to_string(&mut text);
        text
    })
}

/// What a reader thread read, or nothing if it panicked.
fn join(handle: std::thread::JoinHandle<String>) -> String {
    handle.join().unwrap_or_default()
}

/// Is there a `nix` to ask at all?
///
/// So that one config can be written for every machine: a prompt segment or a completion source
/// asks this once rather than paying for a failed spawn on every draw.
pub fn available() -> bool {
    crate::env::builtins::hash_lookup("nix").is_some()
        || std::env::var_os("PATH")
            .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join("nix").is_file()))
}

#[cfg(test)]
#[path = "json/tests.rs"]
mod tests;

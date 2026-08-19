//! What a finished command records, now that recording it happens on another thread.
//!
//! The boundary and the outcome are handed to `track::writer` and written after the prompt has
//! already come back, so this drives a real shell end to end and then reads the store it left
//! behind: the deferred half has to arrive, and it has to say the same thing it always did.
//!
//! **The barrier itself is not what this tests.** `settle` is what makes the deferral safe on the
//! way out, and it is verified where it can be made to fail on purpose — `track::writer`'s own
//! tests, which hold the queue open with a slow job. A writer thread on an idle machine finishes
//! long before a shell has run its exit trap, so these would pass with no barrier at all; what they
//! catch is the deferred writes going missing, landing out of order, or losing the directory.

mod common;

use std::io::Write;
use std::process::{Command, Stdio};

/// Run an interactive shell against a private data directory and answer where its store ended up.
fn interactive_session(lines: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let (home, store, status) = session_ending_however(lines);
    assert!(status.code().is_some(), "the shell was killed, not exited");
    (home, store)
}

/// The same, for the sessions that do not end by `exit` — the ones this file exists to pin.
fn session_ending_however(
    lines: &str,
) -> (
    tempfile::TempDir,
    std::path::PathBuf,
    std::process::ExitStatus,
) {
    let home = tempfile::tempdir().expect("tempdir");
    let data = home.path().join("data");
    std::fs::create_dir_all(home.path().join("config/oslo")).expect("config dir");
    std::fs::create_dir_all(&data).expect("data dir");

    let mut child = Command::new(common::oslo_bin())
        .arg("-i")
        .env("HOME", home.path())
        .env("XDG_DATA_HOME", &data)
        .env("XDG_CONFIG_HOME", home.path().join("config"))
        .env("TERM", "dumb")
        .env("PATH", "/usr/bin:/bin")
        .current_dir(home.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("oslo starts");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(lines.as_bytes())
        .expect("feed the shell");
    let status = child.wait().expect("oslo is reaped");

    let store =
        oslo_base::track::default_path(data.to_str(), home.path().to_str()).expect("a store path");
    (home, store, status)
}

/// Every line the shell accepted, in the order it accepted them.
fn lines_recorded(store: &std::path::Path) -> Vec<String> {
    let track = oslo_base::track::Track::open(store).expect("the store opens");
    let (seen, _) = track.observations(50);
    seen.into_iter().map(|row| row.line).collect()
}

/// **A killed shell keeps the line.** The log row is appended before the command runs, on the prompt
/// thread, and that is the entire reason it is not deferred with everything else: a command that has
/// to be killed is exactly the one you want to find in your history afterwards.
///
/// `docs/features/what-gets-written-down.md` promises this. Deferring the append quietly took it
/// away, and nothing here noticed — so it is asserted now.
#[test]
fn a_killed_shell_still_recorded_the_line_it_was_running() {
    let (_home, store, status) = session_ending_however("echo alpha\nkill -9 $$\n");
    assert!(status.code().is_none(), "the shell was meant to be killed");

    let lines = lines_recorded(&store);
    assert!(
        lines.iter().any(|line| line == "echo alpha"),
        "a killed shell lost the line it had already accepted: {lines:?}"
    );
}

/// **`exec` is an ordinary way out, and has to wait like one.** It never returns to the loop, so it
/// never reaches `settle_stores`; without its own barrier the writer queue is replaced along with
/// the process image and the session loses its tail. `exec $SHELL` after editing a config is the
/// commonest way anyone restarts a shell.
///
/// **Asserted on the status, not the line.** The lines are appended on the prompt thread and would
/// survive an `exec` with no barrier at all; the boundary and the outcome behind them are what the
/// queue is still holding, so a row left at `None` is the shape a missing barrier leaves.
///
/// **This does not prove the barrier.** Measured with it removed, an idle machine drains the queue
/// before `execve` every time — at three commands and at three hundred. The barrier earns its place
/// where the queue is genuinely behind: a large store, a filesystem whose commits block, a loaded
/// box. What this test does catch is the deferred half going missing for any reason that is *not* a
/// race. The barrier's own semantics are pinned in `track::writer`'s tests, which can be made to
/// fail on purpose.
#[test]
fn a_session_that_ends_by_exec_still_wrote_everything() {
    let (_home, store, _) = session_ending_however("echo one\necho two\nexec /bin/true\n");

    let track = oslo_base::track::Track::open(&store).expect("the store opens");
    let (seen, _) = track.observations(50);
    let settled: Vec<(String, Option<i32>)> = seen
        .into_iter()
        .filter(|row| row.line.starts_with("echo "))
        .map(|row| (row.line, row.status))
        .collect();

    assert_eq!(
        settled.len(),
        2,
        "a line went missing entirely: {settled:?}"
    );
    assert!(
        settled.iter().all(|(_, status)| status.is_some()),
        "the writer queue was replaced along with the process image: {settled:?}"
    );
}

/// Every accepted command has its outcome written by the time the process is gone.
#[test]
fn a_session_that_exits_normally_has_written_everything_it_accepted() {
    let (_home, store) = interactive_session("true\nfalse\ntrue\nexit\n");

    let track = oslo_base::track::Track::open(&store).expect("the store opens");
    let (seen, _) = track.observations(50);
    let ran: Vec<&str> = seen.iter().map(|row| row.line.as_str()).collect();
    assert!(
        ran.iter().filter(|line| **line == "true").count() >= 2,
        "the log is missing lines that ran: {ran:?}"
    );

    // The status is the deferred half: the log row is appended before the command runs, and only
    // the outcome written afterwards knows what it exited with.
    let settled: Vec<Option<i32>> = seen
        .iter()
        .filter(|row| row.line == "true" || row.line == "false")
        .map(|row| row.status)
        .collect();
    assert!(
        settled.iter().all(Option::is_some),
        "a command's outcome never landed: {settled:?}"
    );
    assert!(
        settled.contains(&Some(1)),
        "`false` should have been recorded as failing: {settled:?}"
    );
}

/// The directory a command ran in is resolved by the boundary and named by the outcome — the two
/// writes that now share a transaction on a thread that is not the one drawing the prompt.
#[test]
fn a_command_is_still_attributed_to_where_it_ran() {
    let (_home, store) = interactive_session("mkdir -p sub\ncd sub\ntrue\nexit\n");

    let track = oslo_base::track::Track::open(&store).expect("the store opens");
    let (seen, _) = track.observations(50);
    let inside = seen
        .iter()
        .find(|row| row.line == "true")
        .expect("the command ran");
    assert!(
        inside.dir_id != 0,
        "the outcome names no directory, so the boundary's answer was lost"
    );
}

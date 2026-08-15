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
    let status = child.wait().expect("oslo exits");
    assert!(status.code().is_some(), "the shell was killed, not exited");

    let store =
        oslo_base::track::default_path(data.to_str(), home.path().to_str()).expect("a store path");
    (home, store)
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

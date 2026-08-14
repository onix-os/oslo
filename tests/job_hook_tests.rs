//! `on-process-exit` and `on-job-state` — what a status line needs and could not previously get.
//!
//! Both fire from the reaper at a command boundary, which is oslo's existing "next safe moment"
//! model rather than a new one. What is pinned here is that they fire at all, that the payload is
//! the documented one, and that a handler may ask the shell about its jobs — the last of which is
//! the reason the events are queued in the job table and fired after its lock is released.

mod common;

use common::oslo_bin;
use std::process::Command;

/// Run a Lua script through the real binary and answer with its output.
fn lua(script: &str) -> (String, String) {
    let home = tempfile::tempdir().expect("tempdir");
    let file = home.path().join("script.lua");
    std::fs::write(&file, script).expect("write");
    let out = Command::new(oslo_bin())
        .arg(&file)
        .env("HOME", home.path())
        .env("XDG_DATA_HOME", home.path())
        .env("XDG_STATE_HOME", home.path().join("state"))
        .output()
        .expect("spawn oslo");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

/// One process of a job ended, with the status it ended on.
#[test]
fn on_process_exit_fires_with_the_status() {
    let (out, err) = lua(r#"
        oslo.on["on-process-exit"](function(e)
          print("exit status=" .. tostring(e.status))
        end)
        oslo.proc.exec([[sh -c "exit 6" &]])
        oslo.proc.exec("sleep 0.4")
        oslo.proc.exec("true")
        "#);
    assert!(
        out.contains("exit status=6"),
        "the hook did not fire with the status: {out:?} {err}"
    );
}

/// A job that ends is a transition from running to ended, and the payload says which job.
#[test]
fn on_job_state_reports_the_transition() {
    let (out, err) = lua(r#"
        oslo.on["on-job-state"](function(e)
          print("job " .. tostring(e.from) .. "->" .. tostring(e.to) .. " text=" .. tostring(e.text))
        end)
        oslo.proc.exec([[sh -c "exit 0" &]])
        oslo.proc.exec("sleep 0.4")
        oslo.proc.exec("true")
        "#);
    assert!(
        out.contains("running->ended"),
        "no transition reported: {out:?} {err}"
    );
    assert!(out.contains("text=sh -c"), "no command text: {out:?}");
}

/// **A handler may ask the shell about its jobs.**
///
/// The events are recorded while the job table is locked and fired after the lock is released,
/// precisely so that this does not deadlock. A handler calling `oslo.job.list()` is the ordinary
/// case for a status line, not an exotic one.
#[test]
fn a_handler_may_ask_about_jobs_without_deadlocking() {
    let (out, err) = lua(r#"
        oslo.on["on-process-exit"](function(e)
          local jobs = oslo.job.list()
          print("asked and got " .. tostring(#jobs) .. " jobs back")
        end)
        oslo.proc.exec([[sh -c "exit 0" &]])
        oslo.proc.exec("sleep 0.4")
        oslo.proc.exec("true")
        "#);
    assert!(
        out.contains("asked and got"),
        "the handler never returned — a lock it could not take: {out:?} {err}"
    );
}

/// Nothing attached means nothing is built: the reaper runs on every command boundary, so the
/// cost of these hooks when nobody wants them has to be one relaxed load.
#[test]
fn nothing_is_reported_when_nothing_is_attached() {
    let (out, err) = lua(r#"
        oslo.proc.exec([[sh -c "exit 3" &]])
        oslo.proc.exec("sleep 0.4")
        oslo.proc.exec("true")
        print("done")
        "#);
    assert!(out.contains("done"), "{out:?} {err}");
    assert!(!out.contains("status="), "something was announced: {out:?}");
}

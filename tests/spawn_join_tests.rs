//! `job:wait()` and `oslo.settle()` — waiting for a spawn where there is no prompt to wait at.
//!
//! # The bug these close
//!
//! `oslo.spawn` was **silently useless outside the REPL**. A worker queues its result and calls
//! `background::nudge`, which writes one byte to a self-pipe — and only the line editor's `poll`
//! ever watched that pipe. A `.make.lua`, or any `oslo script.lua`, never enters a REPL, so nothing
//! looked: the callback never ran and the script ended having quietly done nothing in parallel.
//!
//! Nothing failed. That is what made it worth a test rather than a fix.
//!
//! The first test below fails against the unpatched binary by hanging until the harness gives up,
//! because the result it waits for was never deliverable.

mod common;

use common::oslo_bin;
use std::process::{Command, Stdio};
use std::time::Instant;

// **The spawned commands close their own stderr.** `oslo.spawn` inherits it deliberately, so a
// background job's complaints reach the terminal — but here that terminal is the pipe this test
// reads, and an orphaned `sleep 30` holding it open makes every timeout look like it was ignored.

fn lua(source: &str) -> (String, std::time::Duration) {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("probe.lua");
    std::fs::write(&file, source).expect("write");
    let began = Instant::now();
    let out = Command::new(oslo_bin())
        .arg(&file)
        .env("HOME", dir.path())
        .env("XDG_DATA_HOME", dir.path())
        .env("XDG_CONFIG_HOME", dir.path().join("config"))
        .stdin(Stdio::null())
        .output()
        .expect("spawn oslo");
    (
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
        began.elapsed(),
    )
}

/// **The headline.** A script can wait for what it spawned and read the result.
#[test]
fn a_script_can_wait_for_what_it_spawned() {
    let (said, _) = lua(r#"
local job = oslo.spawn{ "sh", "-c", "echo hello" }
local out, status = job:wait(10000)
print("out=[" .. tostring(out):gsub("%s+$","") .. "] status=" .. tostring(status))
"#);
    assert!(said.contains("out=[hello]"), "{said}");
    assert!(said.contains("status=0"), "{said}");
}

/// And the callback still runs — `on_exit` promising "always" must not become conditional on
/// whether somebody also joined.
#[test]
fn on_exit_still_fires_for_a_job_that_was_waited_on() {
    let (said, _) = lua(r#"
local job = oslo.spawn{ "sh", "-c", "echo hi", on_exit = function(out, status)
  print("callback=" .. out:gsub("%s+$",""))
end }
local out = job:wait(10000)
print("returned=" .. out:gsub("%s+$",""))
"#);
    assert!(said.contains("callback=hi"), "on_exit was skipped\n{said}");
    assert!(said.contains("returned=hi"), "{said}");
}

/// `oslo.settle` is the one a recipe wants: start several, then wait for all of them.
#[test]
fn settle_waits_for_every_outstanding_spawn() {
    let (said, _) = lua(r#"
local seen = 0
for i = 1, 3 do
  oslo.spawn{ "sh", "-c", "sleep 0.1; echo " .. i, on_exit = function() seen = seen + 1 end }
end
local r = oslo.settle{ timeout_ms = 15000 }
print("settled=" .. tostring(r.settled) .. " fired=" .. r.fired .. " left=" .. r.outstanding)
print("seen=" .. seen)
"#);
    assert!(said.contains("settled=true"), "{said}");
    assert!(said.contains("fired=3"), "{said}");
    assert!(said.contains("left=0"), "{said}");
    assert!(said.contains("seen=3"), "the callbacks did not run\n{said}");
}

/// **They really do overlap.** Three 300 ms sleeps in parallel finish in well under 900 ms, which
/// is what makes this worth having rather than a loop of `oslo.run`.
#[test]
fn spawns_run_at_the_same_time() {
    let (said, took) = lua(r#"
for i = 1, 3 do oslo.spawn{ "sleep", "0.3" } end
print("settled=" .. tostring(oslo.settle{ timeout_ms = 15000 }.settled))
"#);
    assert!(said.contains("settled=true"), "{said}");
    assert!(
        took < std::time::Duration::from_millis(2500),
        "they ran one after another: {took:?}\n{said}"
    );
}

/// Nothing outstanding is not a special case — it answers at once and says so.
#[test]
fn settle_with_nothing_running_answers_immediately() {
    let (said, took) = lua(r#"
local r = oslo.settle()
print("settled=" .. tostring(r.settled) .. " fired=" .. r.fired)
"#);
    assert!(said.contains("settled=true fired=0"), "{said}");
    assert!(took < std::time::Duration::from_secs(5), "{took:?}");
}

#[test]
fn a_deadline_that_passes_is_reported_rather_than_waited_through() {
    let (said, took) = lua(r#"
oslo.spawn{ "sh", "-c", "exec 2>&-; sleep 30" }
local r = oslo.settle{ timeout_ms = 300 }
print("settled=" .. tostring(r.settled) .. " left=" .. r.outstanding)
"#);
    assert!(said.contains("settled=false"), "{said}");
    assert!(said.contains("left=1"), "{said}");
    assert!(
        took < std::time::Duration::from_secs(10),
        "the timeout was ignored: {took:?}\n{said}"
    );
}

#[test]
fn waiting_past_a_deadline_answers_nil_and_why() {
    let (said, took) = lua(r#"
local job = oslo.spawn{ "sh", "-c", "exec 2>&-; sleep 30" }
local out, why = job:wait(300)
print("out=" .. tostring(out) .. " why=" .. tostring(why))
"#);
    assert!(said.contains("out=nil"), "{said}");
    assert!(said.contains("why=timed out"), "{said}");
    assert!(
        took < std::time::Duration::from_secs(10),
        "{took:?}\n{said}"
    );
}

/// A cancelled job is not something to block on — it says so instead of waiting out the deadline.
#[test]
fn waiting_on_a_cancelled_job_answers_at_once() {
    let (said, took) = lua(r#"
local job = oslo.spawn{ "sh", "-c", "exec 2>&-; sleep 30" }
job:cancel()
local out, why = job:wait(30000)
print("out=" .. tostring(out) .. " why=" .. tostring(why))
"#);
    assert!(said.contains("out=nil"), "{said}");
    assert!(
        took < std::time::Duration::from_secs(10),
        "it blocked on a job nobody will deliver: {took:?}\n{said}"
    );
}

/// The status of something that failed reaches the caller, not just its output.
#[test]
fn a_failing_command_reports_its_status() {
    let (said, _) = lua(r#"
local _, status = oslo.spawn{ "sh", "-c", "exit 3" }:wait(10000)
print("status=" .. tostring(status))
"#);
    assert!(said.contains("status=3"), "{said}");
}

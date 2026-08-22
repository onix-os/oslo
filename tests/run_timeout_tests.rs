//! `oslo.run{ timeout_ms = n }` — a command that may not take forever.
//!
//! # The case it is for
//!
//! A `.env.lua` runs on the `cd` path, synchronously, before the prompt is drawn. The things it
//! naturally asks are the slow ones: `git ls-remote`, a cold `nix eval`, a `stat` on an NFS mount
//! that has gone away. One of those hangs **every `cd` into that subtree**, with no way out but
//! SIGINT, and no way for the file to have said "give up after a second".
//!
//! # Two things this had to get right, and one it got wrong first
//!
//! * A deadline forces the forking path. Without one, a command with nothing to capture runs *in
//!   this shell* — which is what makes `sh.cd("/tmp")` move it — and there is nothing there to
//!   interrupt. So a timed non-capturing call runs in a child and no longer moves the shell.
//! * The child gets its own process group, because the direct child is oslo running a subshell and
//!   the command itself may be one level further down.
//!
//! And the one that took the longest to find: **the drain loop read every open stream after
//! `poll`**, not only the ready ones. With both streams captured and one silent, the read on the
//! other blocked until the command exited — so the first version reported `timed_out` at 300 ms and
//! then took the full ten seconds to return. The unbounded `drain` has the same shape and is safe
//! only because it has no deadline to miss.

mod common;

use common::oslo_bin;
use std::process::{Command, Stdio};
use std::time::Instant;

fn lua(source: &str) -> (String, std::time::Duration) {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("probe.lua");
    std::fs::write(&file, source).expect("write");
    let began = Instant::now();
    let out = Command::new(oslo_bin())
        .arg(&file)
        .env("HOME", dir.path())
        .env("XDG_DATA_HOME", dir.path())
        .stdin(Stdio::null())
        .output()
        .expect("spawn oslo");
    let took = began.elapsed();
    (
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
        took,
    )
}

/// A command that finishes is untouched, and says it did not time out.
#[test]
fn a_command_that_finishes_reports_no_timeout() {
    let (said, _) = lua(r#"
local r = oslo.run{"sh","-c","echo quick", capture = true, timeout_ms = 5000}
print("timed_out=" .. tostring(r.timed_out) .. " status=" .. r.status .. " out=" .. r.out:gsub("%s+$",""))
"#);
    assert!(said.contains("timed_out=false"), "{said}");
    assert!(said.contains("status=0"), "{said}");
    assert!(said.contains("out=quick"), "{said}");
}

/// **The one that matters, and the one the first version failed.** A slow command is stopped at the
/// deadline, and the whole call returns then — not when the command would have finished anyway.
#[test]
fn a_slow_command_is_stopped_at_the_deadline() {
    let (said, took) = lua(r#"
local r = oslo.run{"sh","-c","echo before; sleep 30", capture = true, timeout_ms = 300}
print("timed_out=" .. tostring(r.timed_out) .. " kept=[" .. r.out:gsub("%s+$","") .. "]")
"#);
    assert!(said.contains("timed_out=true"), "{said}");
    assert!(
        took < std::time::Duration::from_secs(10),
        "the call waited for the command anyway: {took:?}\n{said}"
    );
}

/// Whatever it wrote before the deadline is kept. A timeout that also discarded the output would
/// make the flag useless for the diagnostic case.
#[test]
fn output_written_before_the_deadline_survives() {
    let (said, _) = lua(r#"
local r = oslo.run{"sh","-c","echo before; sleep 30", capture = true, timeout_ms = 300}
print("kept=[" .. r.out:gsub("%s+$","") .. "]")
"#);
    assert!(said.contains("kept=[before]"), "{said}");
}

/// The status says it was signalled, so a caller can tell a timeout from an ordinary failure even
/// without reading `timed_out`.
#[test]
fn the_status_shows_the_signal() {
    let (said, _) = lua(r#"
local r = oslo.run{"sh","-c","sleep 30", capture = true, timeout_ms = 200}
print("status=" .. r.status .. " signal=" .. tostring(r.signal))
"#);
    assert!(
        said.contains("signal=15"),
        "not terminated by SIGTERM\n{said}"
    );
}

/// **A command that ignores TERM is still collected**, which is what `on_timeout = "KILL"` is for.
#[test]
fn kill_reaches_a_command_that_ignores_term() {
    let (said, took) = lua(r#"
local r = oslo.run{"sh","-c","trap '' TERM; sleep 30", capture = true,
                   timeout_ms = 300, on_timeout = "KILL"}
print("timed_out=" .. tostring(r.timed_out))
"#);
    assert!(said.contains("timed_out=true"), "{said}");
    assert!(
        took < std::time::Duration::from_secs(10),
        "KILL did not reach it: {took:?}\n{said}"
    );
}

/// `timed_out` is always present, never `nil`. A caller asks it unconditionally, and `nil` would
/// read as "no" while meaning "nobody set a deadline".
#[test]
fn timed_out_is_present_even_without_a_deadline() {
    let (said, _) = lua(r#"
local r = oslo.run{"true", capture = true}
print("type=" .. type(r.timed_out) .. " value=" .. tostring(r.timed_out))
"#);
    assert!(said.contains("type=boolean"), "{said}");
    assert!(said.contains("value=false"), "{said}");
}

/// A misspelt `on_timeout` is a caller mistake, so it raises and names the two it accepts.
#[test]
fn an_unknown_on_timeout_is_refused() {
    let (said, _) = lua(r#"
local ok, err = pcall(function() oslo.run{"true", timeout_ms = 10, on_timeout = 5} end)
print("refused=" .. tostring(not ok))
print("names_them=" .. tostring(tostring(err):find("KILL") ~= nil))
"#);
    assert!(said.contains("refused=true"), "{said}");
    assert!(said.contains("names_them=true"), "{said}");
}

/// **A deadline forces a child**, so a non-capturing timed call no longer moves this shell. That is
/// a behaviour change and is pinned rather than left to be discovered.
#[test]
fn a_timed_run_is_a_child_even_with_nothing_captured() {
    let (said, _) = lua(r#"
local before = oslo.fs.cwd()
oslo.run{"cd", "/tmp"}
print("untimed_moved=" .. tostring(oslo.fs.cwd() ~= before))
oslo.run{"cd", "/", timeout_ms = 1000}
print("timed_moved=" .. tostring(oslo.fs.cwd() == "/"))
"#);
    assert!(
        said.contains("timed_moved=false"),
        "a timed run moved the shell, so it did not fork\n{said}"
    );
}

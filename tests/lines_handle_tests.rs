//! `oslo.lines` — the two things it was handed and threw away.
//!
//! # What this exists to catch
//!
//! Both defects were arguments that existed at the call site and were dropped one line later, which
//! is the cheapest kind of bug to write and the hardest to notice:
//!
//! * **stderr never reached the pipe.** `spawn_reading_streams(env, argv, merge_stderr)` had been
//!   there all along and the `keep` builtin already passed `true`; the binding called the
//!   `merge_stderr = false` wrapper. So `for line in oslo.lines{"cargo","build"} do` saw *nothing* —
//!   cargo writes its progress and its errors to stderr — and the module doc names `cargo build` as
//!   the motivating case.
//! * **the exit status was reaped and discarded.** `finish` called `argv::reap`, which answers an
//!   `i32`, as a bare statement. A config could stream a build to its end and had no way to ask
//!   whether it succeeded.
//!
//! Nothing walked the surface, so neither showed up. `tests/lua_api_surface_tests.rs` now does.

mod common;

use common::oslo_bin;
use std::process::{Command, Stdio};

/// Run a Lua chunk through the real binary. Answers stdout and stderr separately, because half of
/// what is being tested here is *which* of the two a line came out on.
fn lua(source: &str) -> (String, String) {
    let dir = tempfile::tempdir().expect("tempdir");
    let file = dir.path().join("probe.lua");
    std::fs::write(&file, source).expect("write");
    let out = Command::new(oslo_bin())
        .arg(&file)
        .env("HOME", dir.path())
        .env("XDG_DATA_HOME", dir.path())
        .env("XDG_CONFIG_HOME", dir.path().join("config"))
        .stdin(Stdio::null())
        .output()
        .expect("spawn oslo");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

/// **The motivating case.** Without `stderr = true` the pipe carries stdout only, which is the
/// POSIX default and right — but it is why a `cargo build` loop looked empty.
#[test]
fn stderr_stays_off_the_pipe_by_default() {
    let (out, err) = lua(r#"
for line in oslo.lines{"sh", "-c", "echo out; echo err >&2"} do print("got:" .. line) end
"#);
    assert!(
        out.contains("got:out"),
        "stdout missing from the pipe\n{out}"
    );
    assert!(
        !out.contains("got:err"),
        "stderr reached the pipe without being asked\n{out}"
    );
    assert!(
        err.contains("err"),
        "stderr should still reach the terminal\n{err}"
    );
}

#[test]
fn stderr_is_interleaved_when_asked() {
    let (out, _) = lua(r#"
for line in oslo.lines{"sh", "-c", "echo out; echo err >&2", stderr = true} do print("got:" .. line) end
"#);
    assert!(out.contains("got:out"), "{out}");
    assert!(out.contains("got:err"), "stderr was not merged\n{out}");
}

/// One pipe, so the order the command wrote them is the order they arrive. Two pipes drained
/// separately could not promise this, which is why `merge_stderr` is a `dup2` and not a second read.
#[test]
fn the_merged_streams_keep_their_order() {
    let (out, _) = lua(r#"
for line in oslo.lines{"sh", "-c", "echo one; echo two >&2; echo three", stderr = true} do
  print("got:" .. line)
end
"#);
    let at = |needle: &str| {
        out.find(needle)
            .unwrap_or_else(|| panic!("{needle}\n{out}"))
    };
    assert!(at("got:one") < at("got:two"), "{out}");
    assert!(at("got:two") < at("got:three"), "{out}");
}

#[test]
fn a_loop_that_runs_out_reports_the_status() {
    let (out, _) = lua(r#"
local h = oslo.lines{"sh", "-c", "echo x; exit 3"}
for _ in h do end
print("status=" .. tostring(h:status()))
"#);
    assert!(out.contains("status=3"), "{out}");
}

#[test]
fn a_command_that_succeeds_reports_zero() {
    let (out, _) = lua(r#"
local h = oslo.lines{"sh", "-c", "echo x"}
for _ in h do end
print("status=" .. tostring(h:status()))
"#);
    assert!(out.contains("status=0"), "{out}");
}

/// `nil` is "nobody has waited yet", not "still running" — asking does not block. The distinction
/// matters because a blocking `status()` would make this two APIs.
#[test]
fn status_is_nil_until_the_child_is_collected() {
    let (out, _) = lua(r#"
local h = oslo.lines{"sh", "-c", "echo one; echo two"}
print("before=" .. tostring(h:status()))
for _ in h do end
print("after=" .. tostring(h:status()))
"#);
    assert!(out.contains("before=nil"), "{out}");
    assert!(out.contains("after=0"), "{out}");
}

/// **`status` is a field, not a verb, and this is the test that proves it matters.**
///
/// Every verb on a handle refuses after `close()`. Registered that way, `status` would raise on
/// exactly the sequence that has an answer — closing is how a caller who broke out of the loop
/// waits for the child.
#[test]
fn status_is_readable_after_close() {
    let (out, err) = lua(r#"
local h = oslo.lines{"sh", "-c", "echo one; echo two"}
h:close()
print("after_close=" .. tostring(h:status()))
"#);
    assert!(
        !err.contains("already"),
        "close made status refuse — it was registered as a verb\n{err}"
    );
    assert!(
        !out.contains("after_close=nil"),
        "no status after close\n{out}"
    );
}

/// **Closing early answers 141 only when the child was still writing.**
///
/// The read end goes first, so a command mid-output takes a `SIGPIPE` on its next write — what
/// `cmd | head -1` does. This is the case people will actually hit, since a command worth streaming
/// is a command producing output.
#[test]
fn closing_a_writing_child_answers_the_signal() {
    let (out, _) = lua(r#"
local h = oslo.lines{"sh", "-c", "while true; do echo tick; done"}
print("first=" .. h())
h:close()
print("status=" .. tostring(h:status()))
"#);
    assert!(out.contains("first=tick"), "{out}");
    assert!(
        out.contains("status=141"),
        "expected 128+SIGPIPE for a child still writing\n{out}"
    );
}

/// **And a quiet child is waited for, not cancelled.** It never notices the closed pipe, so the
/// wait runs to its natural end — which means `close()` on a sleeping command *blocks*. Pinned
/// because the obvious reading of "close" is "cancel", and it is not.
#[test]
fn closing_a_quiet_child_waits_for_its_real_status() {
    let (out, _) = lua(r#"
local h = oslo.lines{"sh", "-c", "echo one; sleep 1; exit 7"}
print("first=" .. h())
h:close()
print("status=" .. tostring(h:status()))
"#);
    assert!(out.contains("first=one"), "{out}");
    assert!(
        out.contains("status=7"),
        "a quiet child should be waited for, not signalled\n{out}"
    );
}

/// A key that means nothing on this call is refused, not dropped. `oslo.run` captures the streams
/// separately and has `capture_err`; silently ignoring `stderr` there would leave the caller
/// believing they had asked for something.
#[test]
fn run_and_pipe_refuse_the_stderr_key() {
    let (out, _) = lua(r#"
local ok, err = pcall(function() oslo.run{"true", stderr = true} end)
print("run_refused=" .. tostring(not ok))
print("names_the_right_key=" .. tostring(tostring(err):find("capture_err") ~= nil))
local ok2 = pcall(function() oslo.pipe({"true", stderr = true}) end)
print("pipe_refused=" .. tostring(not ok2))
"#);
    assert!(out.contains("run_refused=true"), "{out}");
    assert!(out.contains("names_the_right_key=true"), "{out}");
    assert!(out.contains("pipe_refused=true"), "{out}");
}

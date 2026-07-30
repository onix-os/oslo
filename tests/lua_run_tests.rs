//! The argv call model: `oslo.run`, `oslo.pipe` and the `sh` sugar.
//!
//! The property worth testing hardest is that an argument is *never* re-interpreted. A filename
//! holding a space, a `*` or a `;` has to reach the command as one argument, because the whole
//! reason this API exists is that string-concatenated commands cannot promise that.
//!
//! Every case runs the real binary rather than driving `LuaEngine` in-process. Capturing output
//! forks, and forking from a libtest worker forks a *multi-threaded* process: the child gets one
//! thread and any lock another thread happened to hold — `stdout`'s, most easily — is held for
//! ever. That is the same hazard R10.3 moved the subshell tests out for, and it showed up here as
//! a test that simply never finished.

mod common;

use common::oslo_bin;
use std::process::{Command, Stdio};

/// Run a Lua chunk as a script and return its stdout, trimmed.
#[track_caller]
fn lua(script: &str) -> String {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("case.lua");
    std::fs::write(&path, script).expect("write script");
    let output = Command::new(oslo_bin())
        .arg(&path)
        .current_dir(dir.path())
        .env("HOME", dir.path())
        .env_remove("ENV")
        .stdin(Stdio::null())
        .output()
        .expect("spawn oslo");
    assert!(
        output.status.success(),
        "script failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout)
        .trim_end()
        .to_string()
}

/// Run a chunk that is expected to fail, and return its stderr.
#[track_caller]
fn lua_error(script: &str) -> String {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("case.lua");
    std::fs::write(&path, script).expect("write script");
    let output = Command::new(oslo_bin())
        .arg(&path)
        .current_dir(dir.path())
        .stdin(Stdio::null())
        .output()
        .expect("spawn oslo");
    assert!(!output.status.success(), "the script was expected to fail");
    String::from_utf8_lossy(&output.stderr).to_string()
}

#[test]
fn run_reports_status_and_captures_output() {
    let out = lua(r#"
        local r = oslo.run{"echo", "hello", capture = true}
        print(r.status, r.ok, r.out)
    "#);
    assert_eq!(out, "0\ttrue\thello");
}

#[test]
fn a_failing_command_answers_rather_than_raising() {
    // Not an error: a command that fails is ordinary, and a script that wants an exception writes
    // `assert(r.ok)` itself.
    let out = lua(r#"
        local r = oslo.run{"false"}
        print(r.status, r.ok)
    "#);
    assert_eq!(out, "1\tfalse");
}

#[test]
fn an_argument_is_never_re_interpreted() {
    // The point of the whole API. Each of these would break a string-concatenated command: the
    // space would split, the `*` would glob, the `;` would end the command.
    for hostile in ["a b", "*", "a;echo pwned", "$(echo pwned)", "'quoted'", "~"] {
        let out = lua(&format!(
            r#"
            local r = oslo.run{{"printf", "[%s]", {arg:?}, capture = true}}
            print(r.out)
            "#,
            arg = hostile
        ));
        assert_eq!(out, format!("[{hostile}]"), "for {hostile:?}");
    }
}

#[test]
fn capture_is_opt_in_and_distinguishes_empty_from_absent() {
    // No capture at all: `out` is nil, not "". "The command printed nothing" and "nobody was
    // listening" are different facts.
    assert_eq!(
        lua(r#"local r = oslo.run{"true"} print(type(r.out))"#),
        "nil"
    );
    assert_eq!(
        lua(
            r#"local r = oslo.run{"true", capture = true} print(type(r.out), "[" .. r.out .. "]")"#
        ),
        "string\t[]"
    );
}

#[test]
fn the_two_streams_are_separable() {
    let both = lua(r#"
        local r = oslo.run{"sh", "-c", "echo out; echo err >&2", capture = true}
        print(r.out, r.err)
    "#);
    assert_eq!(both, "out\terr");

    // Capturing only stderr leaves stdout alone — it goes to the terminal, so it shows up in the
    // script's own output rather than in `r.out`.
    let only_err = lua(r#"
        local r = oslo.run{"sh", "-c", "echo loose; echo caught >&2", capture_err = true}
        print(type(r.out), r.err)
    "#);
    assert_eq!(only_err, "loose\nnil\tcaught");
}

/// A command writing more than a pipe buffer to *both* streams must not deadlock.
///
/// Draining one stream to the end before starting on the other is the natural implementation and
/// hangs here: the child blocks writing to the pipe nobody is reading, and the parent blocks
/// reading the pipe the child will never finish.
#[test]
fn both_streams_can_exceed_a_pipe_buffer() {
    let out = lua(r#"
        local r = oslo.run{
            "sh", "-c",
            "i=0; while [ $i -lt 4000 ]; do echo aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa; " ..
            "echo bbbbbbbbbbbbbbbbbbbbbbbbbbbbbb >&2; i=$((i+1)); done",
            capture = true,
        }
        print(#r.out, #r.err)
    "#);
    // 4000 lines of 30 characters plus a newline each, less the one trailing newline that is
    // stripped: well past the 64 KiB a pipe will hold.
    assert_eq!(out, "123999\t123999");
}

#[test]
fn a_command_that_runs_in_this_shell_changes_it() {
    // No capture means no fork, which is what makes `cd` through this API persist — the same
    // reason `cd` is a builtin rather than a program.
    let out = lua(r#"
        oslo.run{"cd", "/tmp"}
        print(oslo.get_pwd())
    "#);
    assert_eq!(out, "/tmp");
}

#[test]
fn sh_sugar_builds_the_same_call() {
    assert_eq!(lua(r#"print(sh["true"]().ok)"#), "true");
    // A name that does not exist is a command that could not be run, not a nil index.
    assert_eq!(
        lua(r#"print(sh.definitely_not_a_real_command_zz().status)"#),
        "127"
    );
    // And it takes arguments the same way.
    assert_eq!(lua(r#"sh.printf("%s-%s", "a", "b")"#), "a-b");
}

#[test]
fn sh_refuses_an_argument_that_is_not_a_word() {
    let err = lua_error("sh.echo({})");
    assert!(err.contains("not a word"), "{err}");
}

#[test]
fn pipe_feeds_each_stage_into_the_next() {
    let out = lua(r#"
        local r = oslo.pipe(
            {"printf", "b\na\nc\n"},
            {"sort"},
            {"head", "-n", 2, capture = true}
        )
        print((r.out:gsub("\n", ",")))
    "#);
    assert_eq!(out, "a,b");
}

#[test]
fn a_pipeline_reports_its_last_stages_status() {
    assert_eq!(
        lua(r#"print(oslo.pipe({"echo", "x"}, {"false"}).status)"#),
        "1"
    );
}

/// A stage that stops reading must not leave the shell waiting for the one upstream of it.
#[test]
fn a_pipeline_completes_when_a_stage_stops_reading() {
    let out = lua(r#"
        local r = oslo.pipe(
            {"sh", "-c", "i=0; while [ $i -lt 20000 ]; do echo line; i=$((i+1)); done"},
            {"head", "-n", 1, capture = true}
        )
        print(r.out)
    "#);
    assert_eq!(out, "line");
}

#[test]
fn a_signalled_command_reports_the_signal_apart_from_the_status() {
    // `exit 130` and "killed by SIGINT" both report status 130; only `signal` tells them apart.
    let killed = lua(r#"
        local r = oslo.run{"sh", "-c", "kill -INT $$", capture = true}
        print(r.status, r.signal)
    "#);
    assert_eq!(killed, "130\t2");

    let exited = lua(r#"
        local r = oslo.run{"sh", "-c", "exit 130", capture = true}
        print(r.status, r.signal)
    "#);
    assert_eq!(exited, "130\tnil");
}

#[test]
fn an_empty_argv_is_refused_rather_than_silently_doing_nothing() {
    let err = lua_error("oslo.run{}");
    assert!(err.contains("empty"), "{err}");
}

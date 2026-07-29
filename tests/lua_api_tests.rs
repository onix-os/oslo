//! The `oslo.*` API, from the outside.
//!
//! Split from `startup_tests.rs` because these are about Lua as a *language you write programs
//! in* — argv, capturing a command's answer, choosing an exit status — rather than about how the
//! shell starts up. They drive the real binary for the same reason everything else here does: the
//! bindings are installed by `main`, and an in-process test would be checking a different shell.

mod common;

use common::oslo_bin;
use std::path::Path;
use std::process::{Command, Output, Stdio};

fn run(args: &[&str], vars: &[(&str, &str)], home: &Path) -> Output {
    let mut cmd = Command::new(oslo_bin());
    cmd.args(args)
        .env("HOME", home)
        .env_remove("ENV")
        .env_remove("HISTFILE")
        .stdin(Stdio::null());
    for (k, v) in vars {
        cmd.env(k, v);
    }
    cmd.output().expect("spawn oslo")
}

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

/// A `.lua` operand runs as Lua with no flag, and its status is the shell's.
#[test]
fn lua_script_propagates_the_status_of_what_it_ran() {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("s.lua");

    std::fs::write(&script, "oslo.exec('false')\n").unwrap();
    let failed = run(&[script.to_str().unwrap()], &[], dir.path());
    assert_eq!(
        failed.status.code(),
        Some(1),
        "a failing oslo.exec must show"
    );

    std::fs::write(&script, "oslo.exec('true')\n").unwrap();
    let ok = run(&[script.to_str().unwrap()], &[], dir.path());
    assert_eq!(ok.status.code(), Some(0));

    std::fs::write(&script, "oslo.exec('exit 7')\n").unwrap();
    let exited = run(&[script.to_str().unwrap()], &[], dir.path());
    assert_ne!(
        exited.status.code(),
        Some(0),
        "a script that asked to exit non-zero must not report success"
    );
}

#[test]
fn a_broken_lua_script_exits_non_zero_and_names_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("broken.lua");
    std::fs::write(&script, "not lua(((\n").unwrap();

    let o = run(&[script.to_str().unwrap()], &[], dir.path());
    assert_eq!(o.status.code(), Some(1));
    assert!(err(&o).contains("broken.lua"), "{:?}", err(&o));
}

/// Round B: the `oslo.*` surface a Lua program actually needs.
///
/// One test rather than eight, because these are the calls a real script combines in one breath —
/// capture a command, act on the answer, move, glob, exit with a status — and testing them apart
/// would not catch the case where two of them disagree about the shell's state.
#[test]
fn lua_scripts_can_read_argv_capture_output_and_choose_a_status() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("conf")).unwrap();
    for f in ["a.conf", "b.conf", "skip.txt"] {
        std::fs::write(dir.path().join("conf").join(f), "").unwrap();
    }
    let script = dir.path().join("s.lua");
    std::fs::write(
        &script,
        r##"
-- argv, which was `nil` before this round
assert(arg[0]:match("s%.lua$"), "arg[0] should name the script")
assert(#arg == 1 and arg[1] == "release", "arg should carry the operands")
assert(select("#", ...) == 1, "... should carry them too")

-- capture: the answer *and* the status
local r = oslo.capture("echo  spaced out ")
assert(r.out == "spaced out", "got " .. r.out)
assert(r.status == 0)
assert(oslo.capture("exit 7").status == 7, "a failing command must report its status")
assert(oslo.capture("{ echo e >&2; } 2>&1").out == "e", "2>&1 folds stderr in")

-- cwd, and the shell has to agree with it afterwards
assert(oslo.cd(arg[0]:gsub("/[^/]*$", "")))
assert(oslo.get_pwd() == oslo.capture("pwd").out, "shell and Lua disagree about pwd")
local ok, err = oslo.cd("/no/such/place")
assert(ok == nil and err ~= nil, "a failed cd returns nil, message")

-- glob, with an empty table for no matches rather than the pattern back
assert(#oslo.glob("conf/*.conf") == 2)
assert(#oslo.glob("conf/*.nope") == 0, "no matches must not yield the pattern")

-- environment as a table, both directions
oslo.set_var("OSLO_T_LUA_API", "1")
assert(oslo.env()["OSLO_T_LUA_API"] == "1")
oslo.unset("OSLO_T_LUA_API")
assert(oslo.env()["OSLO_T_LUA_API"] == nil)

oslo.exit(42)
print("NOTREACHED")
"##,
    )
    .unwrap();

    let o = run(&[script.to_str().unwrap(), "release"], &[], dir.path());
    assert!(!out(&o).contains("NOTREACHED"), "oslo.exit did not exit");
    assert_eq!(
        o.status.code(),
        Some(42),
        "oslo.exit must set the shell's status; stderr: {:?}",
        err(&o)
    );
}

/// `oslo.exit` has to work from wherever it is called, not just at the top level: the request
/// travels as an error through however many mlua wrappers the call depth adds.
#[test]
fn lua_exit_works_from_inside_a_function() {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("d.lua");
    std::fs::write(&script, "local function bail() oslo.exit(7) end\nbail()\n").unwrap();
    let o = run(&[script.to_str().unwrap()], &[], dir.path());
    assert_eq!(o.status.code(), Some(7), "stderr: {:?}", err(&o));
}

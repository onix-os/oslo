//! Tools a config registers with `oslo.register_tool`, run as commands.
//!
//! **There were none of these, and the feature was half-broken because of it.** A registered tool
//! could only be reached inside a pipeline — `stale | length` ran, `stale` alone answered
//! `command not found` — because the planner engages on a `Sink::Rows`, and rows only ever describe
//! an *edge* between two stages. A pipeline of one has no edge.
//!
//! Nothing noticed, because every tool oslo ships with is also a real program: a bare `ls` or `ps`
//! quietly ran coreutils and looked correct. Only a name a config invented has nothing to fall
//! through to.
//!
//! These drive Lua scripts rather than a config file: `init.lua` is read only by the interactive
//! REPL, and a script reaches the same registration and the same dispatch without needing a pty.

mod common;

use common::oslo_bin;
use std::process::{Command, Stdio};

fn lua(source: &str) -> (String, String) {
    let dir = tempfile::tempdir().expect("temp dir");
    let script = dir.path().join("case.lua");
    std::fs::write(&script, source).expect("write");
    let out = Command::new(oslo_bin())
        .arg(&script)
        .env("HOME", dir.path())
        .env_remove("ENV")
        .current_dir(dir.path())
        .stdin(Stdio::null())
        .output()
        .expect("spawn oslo");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

const GREET: &str = r#"
oslo.register_tool{ name = "greet", produces = "rows",
  rows = function(argv) return { { who = argv[2] or "world", n = 2 } } end }
"#;

#[test]
fn a_registered_tool_runs_on_its_own() {
    let (out, err) = lua(&format!("{GREET}\noslo.proc.exec('greet')"));
    assert!(!err.contains("command not found"), "{err}");
    assert_eq!(out, "world\t2\n");
}

#[test]
fn it_still_runs_inside_a_pipeline() {
    let (out, _) = lua(&format!("{GREET}\noslo.proc.exec('greet | length')"));
    assert_eq!(out, "1\n");
}

#[test]
fn its_arguments_reach_it() {
    let (out, _) = lua(&format!("{GREET}\noslo.proc.exec('greet everyone')"));
    assert_eq!(out, "everyone\t2\n");
}

/// **The regression this fix could most easily cause.** `ls`, `ps` and `df` are registered tools
/// too, and a bare `ls` must stay coreutils — the structured one is what you get by asking for it
/// with a pipe. Only a *config-registered* name engages on its own.
#[test]
fn a_builtin_tool_on_its_own_is_still_the_external_command() {
    let (out, _) = lua(r#"
        oslo.fs.write("only.txt", "")
        oslo.proc.exec("ls")
        "#);
    // coreutils prints one bare name per line — the script itself is in the directory too. The
    // structured tool would draw a `name` header and a column of them instead.
    assert_eq!(
        out, "case.lua\nonly.txt\n",
        "bare ls must not become the row tool"
    );
}

#[test]
fn a_name_nobody_registered_is_still_not_found() {
    let (_, err) = lua("oslo.proc.exec('definitelynotacommand')");
    assert!(err.contains("not found"), "{err}");
}

/// **`accepts = "bytes"` was a declared shape that could not work.**
///
/// The validator took it, and the planner acted on it — `exec::pipeline::structured` tests for
/// `Shape::Bytes` and reads standard input for a bytes-accepting tool at the head of a pipeline —
/// so the bytes really did arrive at `data::tools::run_tool`. They were then dropped one call short
/// of the handler, because `data::custom::Handler` had only two parameters. Measured before the
/// fix: the tool below answered `0 0 false`.
#[test]
fn a_tool_that_accepts_bytes_is_given_them() {
    let (out, err) = lua(r#"
oslo.register_tool{ name = "counted", accepts = "bytes", produces = "rows",
  rows = function(argv, input, bytes)
    local n = 0
    for _ in tostring(bytes or ""):gmatch("[^\n]+") do n = n + 1 end
    return { { lines = n, got = bytes ~= nil } }
  end }
oslo.proc.exec('printf "a\\nb\\nc\\n" | counted | cols lines got')
"#);
    assert!(
        out.contains('3'),
        "the bytes did not reach the tool\n{out}{err}"
    );
    assert!(out.contains("true"), "bytes was nil\n{out}{err}");
}

/// A tool that never asked for bytes is handed `nil`, not an empty string — the same distinction
/// `input` already makes between "given nothing" and "given no rows".
#[test]
fn a_tool_that_did_not_ask_sees_nil_not_an_empty_string() {
    let (out, err) = lua(r#"
oslo.register_tool{ name = "asked", produces = "rows",
  rows = function(argv, input, bytes) return { { got = bytes ~= nil } } end }
oslo.proc.exec("asked | cols got")
"#);
    assert!(out.contains("false"), "{out}{err}");
}

/// **Widening the handler must not break the narrow ones.** Lua ignores arguments a function did
/// not declare, so every `function(argv)` and `function(argv, input)` tool already written keeps
/// running — which is the whole reason the third argument could be added at all.
#[test]
fn tools_declaring_fewer_arguments_still_run() {
    let (out, err) = lua(r#"
oslo.register_tool{ name = "one", rows = function(argv) return { { n = 1 } } end }
oslo.register_tool{ name = "two", accepts = "rows",
  rows = function(argv, input) return { { n = #(input or {}) } } end }
oslo.proc.exec("one | cols n")
oslo.proc.exec("one | two | cols n")
"#);
    assert!(out.contains('1'), "a one-argument tool broke\n{out}{err}");
}

/// A tool that says what its rows have, in the same table it declares its shapes in.
const DECLARED: &str = r#"
oslo.register_tool{ name = "hosts", columns = { "host", "ip" },
  rows = function() return { { host = "alpha", ip = "10.0.0.1" } } end }
"#;

/// The columns it declared are the ones a later stage may name.
#[test]
fn a_declared_column_goes_through() {
    let (out, err) = lua(&format!("{DECLARED}\noslo.proc.exec('hosts | cols host')"));
    assert!(!err.contains("no such column"), "{err}");
    assert_eq!(out, "alpha\n");
}

/// **And a typo is refused before the tool runs**, which is the whole point of a config's tool
/// being able to say: it is the one that might do something on its way to producing rows.
#[test]
fn a_mistyped_column_is_refused_before_the_tool_runs() {
    let source = r#"
oslo.register_tool{ name = "hosts", columns = { "host", "ip" },
  rows = function() print("THE TOOL RAN") return { { host = "a", ip = "b" } } end }
oslo.proc.exec('hosts | cols hsot')
"#;
    let (out, err) = lua(source);
    assert!(err.contains("hsot"), "the column is named: {err}");
    assert!(
        !out.contains("THE TOOL RAN"),
        "the tool must not have run: {out:?}"
    );
}

/// **A tool that does not say is `Unknown`**, and nothing is ever refused on an `Unknown` — so
/// every tool written before `columns` existed behaves exactly as it did.
#[test]
fn a_tool_that_declares_nothing_is_not_judged() {
    let source = r#"
oslo.register_tool{ name = "quiet",
  rows = function() return { { a = 1 } } end }
oslo.proc.exec('quiet | cols a')
"#;
    let (out, err) = lua(source);
    assert!(!err.contains("no such column"), "{err}");
    assert_eq!(out, "1\n");
}

/// A `columns` that is not a list of names is refused by name, for the same reason a typo in
/// `produces` is: a declaration nobody checks can quietly be wrong.
#[test]
fn columns_must_be_a_list_of_names() {
    let (_, err) = lua(
        r#"oslo.register_tool{ name = "x", columns = "host", rows = function() return {} end }"#,
    );
    assert!(err.contains("columns"), "{err}");
}

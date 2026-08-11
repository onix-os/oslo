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
//! These drive Lua scripts rather than a config file: `config.lua` is read only by the interactive
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

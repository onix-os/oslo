//! `on-variable-change` — the four places a variable changes, and the one that comes from elsewhere.
//!
//! # What these are for
//!
//! The hook's `source` field is the whole reason it exists: a variable changes because you
//! typed something here or because another terminal stored a macro, and a plugin usually acts on one
//! and not the other. A test that only proved "the hook fires" would pass with `source` hard-coded,
//! so each case below asserts the *field*, not the event.
//!
//! **The `remote` case is not here.** A script never re-reads the macro store — only the REPL
//! does, before a prompt and before a command — so the arrival can only be staged against a shell
//! that is actually sitting at a prompt. It lives in `idle_wake_tests`, on a pty, beside the test
//! that the same value reaches the prompt itself.

mod common;

use common::oslo_bin;
use std::process::Command;

/// Run a Lua script through the real binary, in its own `HOME`, and answer with its output.
fn lua_in(home: &std::path::Path, script: &str) -> (String, String) {
    let file = home.join("script.lua");
    std::fs::write(&file, script).expect("write");
    let out = Command::new(oslo_bin())
        .arg(&file)
        .env("HOME", home)
        .env("XDG_DATA_HOME", home)
        .env("XDG_CONFIG_HOME", home.join("config"))
        .env("XDG_STATE_HOME", home.join("state"))
        .output()
        .expect("spawn oslo");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

fn lua(script: &str) -> (String, String) {
    let home = tempfile::tempdir().expect("tempdir");
    lua_in(home.path(), script)
}

/// A plain assignment is announced, with the shell scope and this shell as the source.
#[test]
fn an_assignment_is_announced() {
    let (out, err) = lua(r#"
        oslo.on["on-variable-change"](function(e)
          print("saw " .. e.name .. " " .. e.action .. " " .. e.scope .. " " .. e.source)
        end)
        oslo.proc.exec([[GREETING=hello]])
        "#);
    assert!(
        out.contains("saw GREETING set shell local"),
        "the assignment was not announced: {out:?} {err}"
    );
}

/// `export` says the name is exported; a plain assignment says it is not.
#[test]
fn exported_distinguishes_export_from_assignment() {
    let (out, err) = lua(r#"
        oslo.on["on-variable-change"](function(e)
          print(e.name .. "=" .. e.exported)
        end)
        oslo.proc.exec([[PLAIN=1]])
        oslo.proc.exec([[export SHARED=1]])
        "#);
    assert!(
        out.contains("PLAIN=false") && out.contains("SHARED=true"),
        "exported did not distinguish the two: {out:?} {err}"
    );
}

/// `unset` is an erase, not a set with an empty value.
#[test]
fn unset_is_announced_as_an_erase() {
    let (out, err) = lua(r#"
        oslo.on["on-variable-change"](function(e)
          print(e.name .. " " .. e.action)
        end)
        oslo.proc.exec([[DOOMED=1]])
        oslo.proc.exec([[unset DOOMED]])
        "#);
    assert!(
        out.contains("DOOMED erase"),
        "the erase was not announced: {out:?} {err}"
    );
}

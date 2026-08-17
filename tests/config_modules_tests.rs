//! The configuration as a Lua *program*: `require "oslo"`, and a config split across files.
//!
//! Both of these looked like they worked and did not, which is the worst shape a configuration bug
//! takes — the file reads correctly and the setting simply never happens.
//!
//! * `require "oslo"` answered with a **copy** of the table the global names. Every value crossing
//!   the shell↔VM boundary is converted, so each call built a fresh one:
//!   `local oslo = require "oslo"; oslo.completion.max_rows = 42` wrote into something nothing
//!   would ever read.
//! * `package.path` covered `~/.config/oslo/lua/` but not `~/.config/oslo/` itself, so a second
//!   config file beside `init.lua` could not be reached by name. The only way in was
//!   `dofile(oslo.env.get("HOME") .. "/.config/oslo/aliases.lua")` — an absolute path, written out
//!   by hand, in a language that has had a module system for twenty years.

mod common;

use common::oslo_bin;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};

/// Drive the REPL with a throwaway `$HOME`, as `config_source_tests` does.
fn repl(input: &str, home: &Path) -> Output {
    let mut cmd = Command::new(oslo_bin());
    cmd.arg("-i")
        .env("HOME", home)
        // So a case can ask the shell what it *applied*, rather than what the table holds:
        // `oslo config which` is a subcommand of the binary, and the one under test is not the one
        // on `$PATH`.
        .env("OSLO_TEST_BIN", oslo_bin())
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("ENV")
        .env_remove("PS1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("spawn oslo -i");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(input.as_bytes())
        .expect("write to oslo");
    child.wait_with_output().expect("oslo output")
}

/// Write `init.lua`, plus any siblings, into a throwaway config directory.
fn configured(files: &[(&str, &str)]) -> tempfile::TempDir {
    let home = tempfile::tempdir().expect("tempdir");
    let dir = home.path().join(".config/oslo");
    std::fs::create_dir_all(&dir).expect("mkdir");
    for (name, body) in files {
        std::fs::write(dir.join(name), body).expect("write");
    }
    home
}

/// **`require "oslo"` is the global, not a copy of it.**
#[test]
fn requiring_oslo_answers_the_table_the_global_names() {
    let home = configured(&[(
        "init.lua",
        r#"
        local oslo = require "oslo"
        print("SAME:" .. tostring(oslo == _G.oslo))
        "#,
    )]);
    let out = repl("exit\n", home.path());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("SAME:true"),
        "require gave a different table: {text}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// **And a setting written through it actually lands.**
///
/// The identity check above could pass while writes were lost, if the copy were made later. This
/// asks the question the way a configuration does.
#[test]
fn a_setting_written_through_the_module_takes_effect() {
    let home = configured(&[(
        "init.lua",
        r#"
        local oslo = require "oslo"
        oslo.completion.max_rows = 7
        "#,
    )]);
    let out = repl(
        "$OSLO_TEST_BIN config which completion.max_rows\nexit\n",
        home.path(),
    );
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("completion.max_rows = 7"),
        "the setting was written into a copy: {text}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// **A second config file is reached by name, not by absolute path.**
///
/// This is what makes two files one configuration: `prompt.lua` beside `init.lua` is
/// `require "prompt"`, the same way any other Lua program would say it.
#[test]
fn a_sibling_config_file_is_a_module() {
    let home = configured(&[
        (
            "init.lua",
            r#"
            local prompt = require "prompt"
            print("SIBLING:" .. prompt.name())
            "#,
        ),
        (
            "prompt.lua",
            r#"
            local M = {}
            function M.name() return "loaded-by-require" end
            return M
            "#,
        ),
    ]);
    let out = repl("exit\n", home.path());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("SIBLING:loaded-by-require"),
        "the sibling was not requirable: {text}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A sibling module may configure the shell, which is the whole point of splitting one out.
#[test]
fn a_sibling_module_can_configure_the_shell() {
    let home = configured(&[
        ("init.lua", r#"require "settings""#),
        (
            "settings.lua",
            r#"
            local oslo = require "oslo"
            oslo.completion.max_rows = 9
            return {}
            "#,
        ),
    ]);
    let out = repl(
        "$OSLO_TEST_BIN config which completion.max_rows\nexit\n",
        home.path(),
    );
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("completion.max_rows = 9"),
        "a module's settings did not reach the shell: {text}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// The namespaces are modules too, and the same identity rule holds for them.
#[test]
fn each_namespace_is_a_module_of_its_own() {
    let home = configured(&[(
        "init.lua",
        r#"
        local ui = require "oslo.ui"
        print("NS:" .. tostring(ui == _G.oslo.ui))
        "#,
    )]);
    let out = repl("exit\n", home.path());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("NS:true"),
        "oslo.ui came back as a copy: {text}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

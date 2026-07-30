//! Lua globals and shell variables as one namespace.
//!
//! `export one=two` in shell is readable as `one` in Lua on the next line, and `name = "world"`
//! in Lua is readable as `$name` in shell. One namespace, two spellings — which is what makes
//! switching languages mid-session useful rather than merely possible.
//!
//! The rule that keeps it safe is the *lookup order*: `_G` first, the shell second. A script that
//! sets `type=deploy` or `print=/usr/bin/lpr` must not break `type()` and `print()` in Lua.

mod common;

use common::oslo_bin;
use std::process::{Command, Stdio};

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

#[test]
fn a_lua_string_global_becomes_a_shell_variable() {
    let out = lua(r#"
        name = "world"
        print(oslo.capture("echo $name").out)
    "#);
    assert_eq!(out, "world");
}

#[test]
fn a_shell_variable_is_readable_as_a_lua_global() {
    let out = lua(r#"
        oslo.exec("greeting=hello")
        print(greeting)
        oslo.set_var("other", "x")
        print(other)
    "#);
    assert_eq!(out, "hello\nx");
}

#[test]
fn the_environment_a_script_inherits_is_visible_too() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("case.lua");
    std::fs::write(&path, "print(OSLO_SHARED_TEST)\n").unwrap();
    let output = Command::new(oslo_bin())
        .arg(&path)
        .env("OSLO_SHARED_TEST", "from the environment")
        .env_remove("ENV")
        .stdin(Stdio::null())
        .output()
        .expect("spawn oslo");
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim_end(),
        "from the environment"
    );
}

/// The standard library must win, or a shell script could break Lua by naming a variable badly.
#[test]
fn a_shell_variable_cannot_shadow_the_standard_library() {
    let out = lua(r#"
        oslo.exec("type=deploy")
        oslo.exec("print=/usr/bin/lpr")
        -- `_G` is consulted first, so both of these are still the functions.
        print(type(1), type(print))
        -- And the shell still has its own values.
        print(oslo.get_var("type"), oslo.get_var("print"))
    "#);
    assert_eq!(out, "number\tfunction\ndeploy\t/usr/bin/lpr");
}

/// Only strings cross over. A shell variable cannot hold a table.
#[test]
fn a_non_string_global_stays_in_lua() {
    let out = lua(r#"
        config = {retries = 3}
        print(type(config), config.retries)
        -- Nothing was flattened into the shell.
        print(oslo.get_var("config"))
    "#);
    assert_eq!(out, "table\t3\nnil");
}

/// A name lives in exactly one of the two homes, so changing its type moves it.
#[test]
fn a_name_that_changes_type_moves_rather_than_leaving_a_copy() {
    let out = lua(r#"
        x = "a string"
        print(oslo.get_var("x"), type(x))
        x = {1, 2}
        -- The shell variable is gone, not stale.
        print(oslo.get_var("x"), type(x))
        x = "back to a string"
        print(oslo.get_var("x"), type(x))
    "#);
    assert_eq!(
        out,
        "a string\tstring\nnil\ttable\nback to a string\tstring"
    );
}

#[test]
fn assigning_nil_removes_the_name_from_both() {
    let out = lua(r#"
        gone = "here"
        gone = nil
        print(gone, oslo.get_var("gone"))
    "#);
    assert_eq!(out, "nil\tnil");
}

/// `local` is Lua's own, and must not reach the shell at all.
#[test]
fn a_local_stays_private_to_lua() {
    let out = lua(r#"
        local secret = "not exported"
        print(secret, oslo.get_var("secret"))
    "#);
    assert_eq!(out, "not exported\tnil");
}

/// A function defined at the top level is a global, and a global is not a shell variable unless
/// it is a string.
#[test]
fn a_global_function_is_callable_and_invisible_to_the_shell() {
    let out = lua(r#"
        function greet(who) return "hi " .. who end
        print(greet("you"), oslo.get_var("greet"))
    "#);
    assert_eq!(out, "hi you\tnil");
}

/// A number written to a global is a Lua number, not a string, so it stays in `_G`.
#[test]
fn a_number_global_keeps_its_type() {
    let out = lua(r#"
        count = 3
        print(math.type(count), oslo.get_var("count"))
        -- Written as a string, it crosses over and comes back as one.
        text = "3"
        print(math.type(text), oslo.get_var("text"), type(text))
    "#);
    assert_eq!(out, "integer\tnil\nnil\t3\tstring");
}

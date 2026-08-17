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
        print(oslo.proc.capture("echo $name").out)
    "#);
    assert_eq!(out, "world");
}

#[test]
fn a_shell_variable_is_readable_as_a_lua_global() {
    let out = lua(r#"
        oslo.proc.exec("greeting=hello")
        print(greeting)
        oslo.env.set("other", "x")
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
        oslo.proc.exec("type=deploy")
        oslo.proc.exec("print=/usr/bin/lpr")
        -- `_G` is consulted first, so both of these are still the functions.
        print(type(1), type(print))
        -- And the shell still has its own values.
        print(oslo.env.get("type"), oslo.env.get("print"))
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
        print(oslo.env.get("config"))
    "#);
    assert_eq!(out, "table\t3\nnil");
}

/// A name lives in exactly one of the two homes, so changing its type moves it.
///
/// **Both directions.** Going *to* a table always worked; coming back did not — a name that had
/// held a non-string was in `_G` from then on, and `__newindex`, the only thing that sends a
/// string to the shell, does not fire for a name `_G` already has. `x = {}` then `x = "s"` left
/// `$x` unset, and that was written down in two places as a divergence oslo lived with.
///
/// It lives with it no longer: a script's tables are kept *beside* `_G` rather than in it, so the
/// name never lands there and the metamethod keeps firing. See `crates/oslo-luavm/src/globals.rs`.
#[test]
fn a_name_that_changes_type_moves_rather_than_leaving_a_copy() {
    let out = lua(r#"
        x = "a string"
        print(oslo.env.get("x"), type(x))
        x = {1, 2}
        -- The shell variable is gone, not stale.
        print(oslo.env.get("x"), type(x))
        x = "back to a string"
        print(oslo.env.get("x"), type(x))
    "#);
    assert_eq!(
        out,
        "a string\tstring\nnil\ttable\nback to a string\tstring"
    );
}

/// The same round trip through a function, which is the other thing a shell variable cannot hold.
#[test]
fn a_function_global_gives_the_name_back_when_it_becomes_a_string() {
    let out = lua(r#"
        f = function() return 7 end
        print(f(), oslo.env.get("f"))
        f = "now a string"
        print(oslo.env.get("f"), type(f))
        f = nil
        print(oslo.env.get("f"), type(f))
    "#);
    assert_eq!(out, "7\tnil\nnow a string\tstring\nnil\tnil");
}

/// **What keeping them beside `_G` costs**, pinned so it is a decision rather than a surprise.
///
/// A script's own globals are not raw-visible: `rawget` and `pairs` go straight to the table and
/// there is no `__pairs` in Lua 5.4 to intercept them. The standard library is unaffected — those
/// names really are in `_G` — and nothing in oslo enumerates `_G`, which is the reason this is the
/// cheaper half of the trade. See the module docs in `crates/oslo-luavm/src/globals.rs`.
#[test]
fn a_script_global_is_reachable_but_not_raw_visible() {
    let out = lua(r#"
        held = {1}
        -- Ordinary reads find it, through the metatable.
        print("read", held[1], _G.held[1])
        -- Raw ones do not, and neither does walking _G.
        print("raw", rawget(_G, "held"))
        local seen = false
        for name in pairs(_G) do if name == "held" then seen = true end end
        print("walked", seen)
        -- The standard library is still exactly where Lua expects it.
        print("stdlib", rawget(_G, "print") ~= nil)
    "#);
    assert_eq!(out, "read\t1\t1\nraw\tnil\nwalked\tfalse\nstdlib\ttrue");
}

#[test]
fn assigning_nil_removes_the_name_from_both() {
    let out = lua(r#"
        gone = "here"
        gone = nil
        print(gone, oslo.env.get("gone"))
    "#);
    assert_eq!(out, "nil\tnil");
}

/// `local` is Lua's own, and must not reach the shell at all.
#[test]
fn a_local_stays_private_to_lua() {
    let out = lua(r#"
        local secret = "not exported"
        print(secret, oslo.env.get("secret"))
    "#);
    assert_eq!(out, "not exported\tnil");
}

/// A function defined at the top level is a global, and a global is not a shell variable unless
/// it is a string.
#[test]
fn a_global_function_is_callable_and_invisible_to_the_shell() {
    let out = lua(r#"
        function greet(who) return "hi " .. who end
        print(greet("you"), oslo.env.get("greet"))
    "#);
    assert_eq!(out, "hi you\tnil");
}

/// A number written to a global is a Lua number, not a string, so it stays in `_G`.
#[test]
fn a_number_global_keeps_its_type() {
    let out = lua(r#"
        count = 3
        print(math.type(count), oslo.env.get("count"))
        -- Written as a string, it crosses over and comes back as one.
        text = "3"
        print(math.type(text), oslo.env.get("text"), type(text))
    "#);
    assert_eq!(out, "integer\tnil\nnil\t3\tstring");
}

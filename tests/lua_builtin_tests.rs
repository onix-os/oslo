//! `rush.register_builtin` — PLAN R9.8.
//!
//! Before this round the binding accepted a callback, dropped it, and registered the stub
//! `|_, _| Ok(0)` under the name anyway. That made `rush.register_builtin('ls', …)` turn `ls /`
//! into a command that printed nothing and exited 0 — a saboteur, not a no-op. These tests pin
//! the three things that were wrong: the callback runs, its result is the exit status, and a
//! failure inside it is not reported as success.

use rush::env::Environment;
use rush::exec::eval_command_list;
use rush::lua::LuaEngine;
use rush::parser::parse_bash_script;
use std::sync::{Arc, Mutex};

/// A shell with `script` already evaluated by the Lua engine.
///
/// The engine is returned alongside the environment because dropping it would take the Lua state
/// — and every registered callback — with it.
fn shell_with_lua(script: &str) -> (LuaEngine, Arc<Mutex<Environment>>) {
    let env = Arc::new(Mutex::new(Environment::new()));
    let lua = LuaEngine::new().expect("Lua init failed");
    lua.setup_bindings(Arc::clone(&env)).expect("bindings");
    lua.eval_script(script).expect("script");
    (lua, env)
}

fn run(env: &Arc<Mutex<Environment>>, line: &str) -> i32 {
    let ast = parse_bash_script(line).expect("parse");
    let mut guard = env.lock().unwrap();
    eval_command_list(&mut guard, &ast).expect("eval")
}

#[test]
fn a_registered_builtin_runs_and_its_return_value_is_the_status() {
    let (_lua, env) = shell_with_lua(
        r#"
        rush.register_builtin("statuser", function(argv)
            return 7
        end)
        "#,
    );
    assert!(env.lock().unwrap().is_builtin("statuser"));
    assert_eq!(run(&env, "statuser"), 7);
}

/// The callback sees the same argv a builtin written in Rust does: its own name first.
///
/// Asserted inside Lua and reported as the exit status, because the exit status is the only
/// channel out of a builtin that the shell itself defines.
#[test]
fn the_callback_receives_argv_with_its_own_name_first() {
    let (_lua, env) = shell_with_lua(
        r#"
        rush.register_builtin("argviewer", function(argv)
            if table.concat(argv, ",") ~= "argviewer,one,two" then return 99 end
            return #argv
        end)
        "#,
    );
    assert_eq!(run(&env, "argviewer one two"), 3);
}

/// `return` is optional: a builtin that only prints something succeeds, and `false` fails.
#[test]
fn missing_and_boolean_return_values_become_a_status() {
    let (_lua, env) = shell_with_lua(
        r#"
        rush.register_builtin("silent", function(argv) end)
        rush.register_builtin("yes", function(argv) return true end)
        rush.register_builtin("no", function(argv) return false end)
        "#,
    );
    assert_eq!(run(&env, "silent"), 0);
    assert_eq!(run(&env, "yes"), 0);
    assert_eq!(run(&env, "no"), 1);
}

/// An error raised inside the callback must not read as success. It is the user's script that
/// failed, so it becomes a diagnostic plus a non-zero status, not a panic and not `0`.
#[test]
fn an_error_inside_the_callback_is_a_failure_status() {
    let (_lua, env) = shell_with_lua(
        r#"
        rush.register_builtin("exploder", function(argv)
            error("deliberate")
        end)
        "#,
    );
    assert_eq!(run(&env, "exploder"), 1);
}

/// A registered name takes over from the builtin of the same name — the collision case. This is
/// also what the old code could not do at all: `register_builtin('echo', …)` was doubly dead,
/// once because the callback was dropped and once because the dispatcher had its own table.
#[test]
fn a_registered_name_overrides_the_native_builtin() {
    let (_lua, env) = shell_with_lua(
        r#"
        rush.register_builtin("true", function(argv) return 3 end)
        "#,
    );
    assert_eq!(run(&env, "true"), 3);
}

/// A registered builtin comes before `PATH`, like every other builtin. `env` is used because the
/// real program exits 0 and prints, so falling through to `PATH` is unmistakable.
#[test]
fn a_registered_name_is_preferred_over_the_program_on_path() {
    let (_lua, env) = shell_with_lua(
        r#"
        rush.register_builtin("env", function(argv) return 5 end)
        "#,
    );
    assert_eq!(run(&env, "env"), 5);
}

/// The re-entrancy rule, stated as a test so it cannot regress into a hang: the evaluator holds
/// the shell state while the callback runs, so `rush.*` inside a callback raises a Lua error.
/// Before `try_lock` this deadlocked, and an interactive shell simply stopped responding.
#[test]
fn calling_back_into_the_rush_api_errors_instead_of_deadlocking() {
    let (_lua, env) = shell_with_lua(
        r#"
        rush.register_builtin("reenter", function(argv)
            rush.set_var("SHOULD_NOT_HAPPEN", "1")
            return 0
        end)
        "#,
    );
    assert_eq!(run(&env, "reenter"), 1);
    assert_eq!(env.lock().unwrap().get_param("SHOULD_NOT_HAPPEN"), None);
}

/// An empty name is refused at registration rather than inserted into the registry, where it
/// would be a builtin that no command word can ever name.
#[test]
fn an_empty_builtin_name_is_refused() {
    let env = Arc::new(Mutex::new(Environment::new()));
    let lua = LuaEngine::new().expect("Lua init failed");
    lua.setup_bindings(Arc::clone(&env)).expect("bindings");
    let err = lua
        .eval_script(r#"rush.register_builtin("  ", function() end)"#)
        .expect_err("an empty name must be an error");
    assert!(err.to_string().contains("must not be empty"), "{err}");
}

/// PLAN R9.7: the right-prompt API is gone, not merely unused. A script that still calls it gets
/// an error it can see, rather than setting a callback nothing will ever read.
#[test]
fn set_right_prompt_no_longer_exists() {
    let env = Arc::new(Mutex::new(Environment::new()));
    let lua = LuaEngine::new().expect("Lua init failed");
    lua.setup_bindings(Arc::clone(&env)).expect("bindings");
    assert!(
        lua.eval_script("rush.set_right_prompt(function() return '' end)")
            .is_err()
    );
}

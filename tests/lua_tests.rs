//! The `oslo.*` table: one test per binding, each with its failure mode (PLAN R10.2).
//!
//! `register_builtin` has its own suite in `lua_builtin_tests.rs` (R9.8); what is left here is
//! every other binding — `exec`, `get_var`, `set_var`, `get_pwd`, `set_alias`, `get_alias`,
//! `set_prompt` — plus `load_file` and `eval_script` themselves. The failure modes matter as much
//! as the happy paths: a binding that swallows a bad call is an `init.lua` that silently does
//! nothing, which is exactly how the dead `set_right_prompt` survived so long.

use oslo::env::Environment;
use oslo::lua::LuaEngine;
use std::fs;
use std::sync::{Arc, Mutex};

/// An engine with the bindings installed, and the shell state they act on.
///
/// The engine is returned because dropping it takes the Lua state — and anything registered in
/// it — with it.
fn engine() -> (LuaEngine, Arc<Mutex<Environment>>) {
    let env = Arc::new(Mutex::new(Environment::new()));
    let lua = LuaEngine::new().expect("Lua init failed");
    lua.setup_bindings(Arc::clone(&env)).expect("bindings");
    (lua, env)
}

fn param(env: &Arc<Mutex<Environment>>, name: &str) -> Option<String> {
    env.lock().unwrap().get_param(name)
}

// ------------------------------------------------------------------------ oslo.proc.exec

#[test]
fn exec_runs_a_command_and_returns_its_status() {
    let (lua, _env) = engine();
    lua.eval_script(
        r#"
        assert(oslo.proc.exec("true") == 0, "true should be 0")
        assert(oslo.proc.exec("false") == 1, "false should be 1")
        assert(oslo.proc.exec("true && false") == 1, "the list status is the last one")
        assert(oslo.proc.exec("false; true") == 0, "and only the last one")
        "#,
    )
    .expect("exec");
}

#[test]
fn exec_cannot_be_used_to_exit_the_shell() {
    let (lua, _env) = engine();
    // `exit` unwinds the evaluator rather than returning a status, so it reaches Lua as an error
    // instead of ending the process. Pinned because it is the one command whose result a script
    // cannot read: the caller sees a raised error, not the number it asked to exit with.
    let err = lua.eval_script(r#"oslo.proc.exec("exit 7")"#).unwrap_err();
    assert!(err.to_string().contains('7'), "{err}");
}

#[test]
fn exec_acts_on_the_shell_state_the_caller_shares() {
    let (lua, env) = engine();
    // The whole point of the binding: `oslo.proc.exec` is not a subshell, it is *this* shell.
    lua.eval_script(r#"oslo.proc.exec("ZZ_FROM_EXEC=42")"#)
        .unwrap();
    assert_eq!(param(&env, "ZZ_FROM_EXEC").as_deref(), Some("42"));

    lua.eval_script(r#"oslo.proc.exec("false")"#).unwrap();
    assert_eq!(param(&env, "?").as_deref(), Some("1"), "$? is shared too");
}

#[test]
fn exec_reports_unparseable_input_as_a_lua_error() {
    let (lua, _env) = engine();
    // A parse failure is the script's mistake and has no exit status to return; swallowing it
    // would make `oslo.proc.exec` look like it ran something.
    let err = lua
        .eval_script(r#"oslo.proc.exec("if")"#)
        .expect_err("an unfinished command must be an error");
    assert!(err.to_string().contains("Lua error"), "{err}");

    // A *missing* command is not a Lua error: it is a command that ran and failed, like in any
    // shell.
    lua.eval_script(r#"assert(oslo.proc.exec("zz-no-such-command-here") == 127)"#)
        .expect("a missing command is a status, not an error");
}

#[test]
fn exec_without_a_command_string_is_an_error() {
    let (lua, _env) = engine();
    assert!(lua.eval_script("oslo.proc.exec()").is_err());
}

// --------------------------------------------------------------------- oslo.env.get

#[test]
fn get_var_reads_shell_variables_and_special_parameters() {
    let (lua, env) = engine();
    env.lock().unwrap().set_var("ZZ_READ_ME", "value", false);

    lua.eval_script(
        r#"
        assert(oslo.env.get("ZZ_READ_ME") == "value", "plain variable")
        assert(oslo.env.get("?") == "0", "special parameters resolve too")
        "#,
    )
    .expect("get_var");
}

#[test]
fn get_var_answers_nil_for_a_name_that_is_not_set() {
    let (lua, _env) = engine();
    // `nil`, not `""`: a script has to be able to tell "unset" from "set to empty".
    lua.eval_script(
        r#"
        assert(oslo.env.get("ZZ_DEFINITELY_UNSET") == nil, "unset must be nil")
        "#,
    )
    .expect("get_var");
    assert!(lua.eval_script("oslo.env.get()").is_err(), "name required");
}

// --------------------------------------------------------------------- oslo.env.set

#[test]
fn set_var_is_visible_to_the_shell_and_to_lua() {
    let (lua, env) = engine();
    lua.eval_script(
        r#"
        oslo.env.set("ZZ_SET_BY_LUA", "hello")
        assert(oslo.env.get("ZZ_SET_BY_LUA") == "hello", "readable back")
        assert(oslo.proc.exec('[ "$ZZ_SET_BY_LUA" = hello ]') == 0, "visible to commands")
        "#,
    )
    .expect("set_var");
    assert_eq!(param(&env, "ZZ_SET_BY_LUA").as_deref(), Some("hello"));
}

#[test]
fn set_var_needs_both_a_name_and_a_value() {
    let (lua, env) = engine();
    assert!(lua.eval_script(r#"oslo.env.set("ZZ_HALF")"#).is_err());
    assert_eq!(param(&env, "ZZ_HALF"), None, "nothing half-assigned");
}

#[test]
fn set_var_cannot_overwrite_a_readonly_variable() {
    let (lua, env) = engine();
    lua.eval_script(r#"oslo.proc.exec("readonly ZZ_LOCKED=1")"#)
        .unwrap();

    // The binding drops `set_var`'s `false`, so the refusal is silent in Lua — the shell prints
    // the diagnostic. What must not happen is the value changing.
    lua.eval_script(r#"oslo.env.set("ZZ_LOCKED", "2")"#)
        .unwrap();
    assert_eq!(param(&env, "ZZ_LOCKED").as_deref(), Some("1"));
}

// --------------------------------------------------------------------- oslo.sys.pwd

#[test]
fn get_pwd_reports_the_processs_working_directory() {
    let (lua, _env) = engine();
    let cwd = std::env::current_dir()
        .unwrap()
        .to_string_lossy()
        .to_string();
    lua.eval_script(&format!(
        r#"assert(oslo.sys.pwd() == {cwd:?}, "got " .. oslo.sys.pwd())"#
    ))
    .expect("get_pwd");
}

#[test]
fn get_pwd_ignores_pwd_the_variable() {
    let (lua, env) = engine();
    // `$PWD` is a variable anything can write; `get_pwd` answers where the process actually is,
    // which is what a prompt callback needs. (Its own failure mode — an unreadable cwd, where it
    // returns the empty string — cannot be provoked without `chdir`, which is process-wide and
    // would corrupt every other test in this binary.) Set unexported, for the same reason: an
    // exported assignment would write the real environment of the test process.
    env.lock().unwrap().set_var("PWD", "/nowhere-at-all", false);
    lua.eval_script(r#"assert(oslo.sys.pwd() ~= "/nowhere-at-all", oslo.sys.pwd())"#)
        .expect("get_pwd");
    lua.eval_script(r#"assert(oslo.env.get("PWD") == "/nowhere-at-all", "the variable stands")"#)
        .expect("get_var");
}

// ------------------------------------------------------- oslo.env.set_alias / get_alias

#[test]
fn an_alias_set_from_lua_is_the_shells_alias() {
    let (lua, env) = engine();
    lua.eval_script(
        r#"
        oslo.env.set_alias("zzl", "ls -la")
        assert(oslo.env.alias("zzl") == "ls -la", "readable back")
        "#,
    )
    .expect("set_alias");
    assert_eq!(
        env.lock().unwrap().get_alias("zzl").map(str::to_string),
        Some("ls -la".to_string())
    );
}

#[test]
fn get_alias_answers_nil_for_an_unknown_name() {
    let (lua, _env) = engine();
    lua.eval_script(r#"assert(oslo.env.alias("zz-no-such-alias") == nil)"#)
        .expect("get_alias");
    assert!(
        lua.eval_script("oslo.env.alias()").is_err(),
        "name required"
    );
}

#[test]
fn set_alias_needs_a_target() {
    let (lua, env) = engine();
    assert!(lua.eval_script(r#"oslo.env.set_alias("zzhalf")"#).is_err());
    assert!(env.lock().unwrap().get_alias("zzhalf").is_none());
}

// ------------------------------------------------------------------ oslo.ui.prompt

#[test]
fn the_prompt_callback_is_what_render_prompt_returns() {
    let (lua, _env) = engine();
    assert_eq!(lua.render_prompt(), None, "nothing set yet");

    lua.eval_script(r#"oslo.ui.prompt(function() return "oslo$ " end)"#)
        .expect("set_prompt");
    assert_eq!(lua.render_prompt().as_deref(), Some("oslo$ "));

    // The last one registered wins, so an `init.lua` reloaded twice does not stack prompts.
    lua.eval_script(r#"oslo.ui.prompt(function() return "second> " end)"#)
        .unwrap();
    assert_eq!(lua.render_prompt().as_deref(), Some("second> "));
}

#[test]
fn a_prompt_callback_may_use_the_rest_of_the_api() {
    let (lua, _env) = engine();
    lua.eval_script(
        r#"
        oslo.env.set("ZZ_PROMPT_BIT", "x")
        oslo.ui.prompt(function() return oslo.env.get("ZZ_PROMPT_BIT") .. "> " end)
        "#,
    )
    .unwrap();
    // The prompt is rendered outside the evaluator, so nothing holds the shell lock here.
    assert_eq!(lua.render_prompt().as_deref(), Some("x> "));
}

#[test]
fn a_prompt_callback_that_fails_leaves_the_prompt_to_the_shell() {
    let (lua, _env) = engine();
    // A broken prompt must not take the shell down, and must not print a traceback where the
    // prompt goes: `None` means "use the built-in prompt".
    lua.eval_script(r#"oslo.ui.prompt(function() error("boom") end)"#)
        .unwrap();
    assert_eq!(lua.render_prompt(), None);

    lua.eval_script(r#"oslo.ui.prompt(function() return nil end)"#)
        .unwrap();
    assert_eq!(lua.render_prompt(), None);
}

#[test]
fn set_prompt_refuses_anything_that_is_not_a_function() {
    let (lua, _env) = engine();
    assert!(lua.eval_script(r#"oslo.ui.prompt("oslo$ ")"#).is_err());
    assert_eq!(lua.render_prompt(), None, "and nothing was stored");
}

// ------------------------------------------------------------ oslo.register_builtin

#[test]
fn register_builtin_refuses_a_declaration_it_cannot_use() {
    let (lua, env) = engine();
    // The R9.8 suite covers the callback that runs; this is the other half — a bad registration
    // must not leave a name in the builtin registry with nothing behind it.
    //
    // **The two-argument spelling is one of the refusals now.** It was
    // `oslo.register_builtin("hi", function(argv) … end)` and is a table only, because the handler
    // takes a second argument — the shell record — and a positional form has nowhere to say
    // `wants`. Nothing in the tree still uses it; the message names the shape to write instead.
    for bad in [
        r#"oslo.register_builtin("zzbad", function(argv) end)"#,
        r#"oslo.register_builtin("zzbad", "not a function")"#,
        r#"oslo.register_builtin("zzbad")"#,
        r#"oslo.register_builtin{ name = "zzbad" }"#,
        r#"oslo.register_builtin{ name = "zzbad", run = "not a function" }"#,
        r#"oslo.register_builtin{ run = function() end }"#,
        r#"oslo.register_builtin{ name = "zzbad", run = function() end, wants = { "variables" } }"#,
    ] {
        assert!(lua.eval_script(bad).is_err(), "accepted: {bad}");
        assert!(
            !env.lock().unwrap().is_builtin("zzbad"),
            "a refused declaration left the name registered: {bad}"
        );
    }
}

// --------------------------------------------------------------- the absent bindings

#[test]
fn there_is_no_right_prompt_binding() {
    let (lua, _env) = engine();
    // PLAN R9.7: it was registered and never rendered. A script that still calls it must fail
    // loudly rather than configure something nothing reads.
    lua.eval_script(r#"assert(oslo.set_right_prompt == nil)"#)
        .expect("set_right_prompt must be absent");
}

#[test]
fn the_oslo_table_does_not_exist_until_the_bindings_are_installed() {
    let lua = LuaEngine::new().expect("Lua init failed");
    lua.eval_script("assert(oslo == nil)")
        .expect("no bindings, no table");
    assert!(lua.eval_script(r#"oslo.proc.exec("true")"#).is_err());
}

// ------------------------------------------------------------------ eval / load_file

#[test]
fn load_file_runs_a_script_from_disk() {
    let (lua, env) = engine();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("init.lua");
    fs::write(
        &path,
        "oslo.env.set_alias('zzfile', 'echo from-file')\noslo.env.set('ZZ_FROM_FILE', 'yes')\n",
    )
    .unwrap();

    lua.load_file(path.to_str().unwrap()).expect("load_file");
    assert_eq!(param(&env, "ZZ_FROM_FILE").as_deref(), Some("yes"));
    assert_eq!(
        env.lock().unwrap().get_alias("zzfile").map(str::to_string),
        Some("echo from-file".to_string())
    );
}

#[test]
fn load_file_reports_a_missing_file_rather_than_ignoring_it() {
    let (lua, _env) = engine();
    let err = lua
        .load_file("/nonexistent-dir-zz/init.lua")
        .expect_err("a missing file must be an error");
    // The message has to name the file: an `init.lua` that never loaded is otherwise invisible.
    assert!(
        err.to_string().contains("No such file") || err.to_string().contains("nonexistent-dir-zz"),
        "{err}"
    );
}

#[test]
fn an_error_in_a_loaded_file_points_at_that_file() {
    let (lua, _env) = engine();
    let dir = tempfile::tempdir().unwrap();

    let broken = dir.path().join("broken.lua");
    fs::write(&broken, "this is not lua(\n").unwrap();
    let err = lua.load_file(broken.to_str().unwrap()).unwrap_err();
    assert!(err.to_string().contains("broken.lua"), "{err}");

    // The same for a failure at run time: the chunk is named after the file, so the traceback
    // points at the user's script and not at the Rust call site that loaded it.
    let raising = dir.path().join("raising.lua");
    fs::write(&raising, "error('deliberate')\n").unwrap();
    let err = lua.load_file(raising.to_str().unwrap()).unwrap_err();
    assert!(err.to_string().contains("raising.lua"), "{err}");
    assert!(err.to_string().contains("deliberate"), "{err}");
}

#[test]
fn eval_script_reports_a_syntax_error() {
    let (lua, _env) = engine();
    assert!(lua.eval_script("this is not lua(").is_err());
    // And a valid script still runs afterwards: one bad chunk does not poison the interpreter.
    lua.eval_script("oslo.env.set('ZZ_AFTER_ERROR', '1')")
        .expect("the engine still works");
}

/// **One runtime, one way of printing a number.**
///
/// The VM's `tostring` and the shared [`oslo_base::value::Number`] formatter both claim to be Lua's
/// `%.14g`, and they disagreed: the shared one discarded the mantissa it had computed and fell back
/// to Rust's shortest round-tripping form. So `tostring(0.1+0.2)` was `0.3` while everything going
/// through a `Value` — `sh.echo`, prompt segment fields, dropdown lines, structured-pipeline
/// columns — printed `0.30000000000000004`.
///
/// The VM renders the list and hands it back through a shell variable; Rust renders the same list
/// through `Number` and compares. Neither side is a written-down expectation, so neither can drift
/// without this failing.
///
/// `oslo.json.encode` is deliberately *not* one of the paths checked: JSON is a serialisation
/// format and `serde_json` writes the shortest form that round-trips, which is the right answer
/// there and a different question from how a number is shown to a person.
#[test]
fn a_number_prints_the_same_through_the_vm_and_through_a_value() {
    use oslo_base::value::Number;

    // Written once in each language, because comparing the two renderings is the whole point.
    let expressions = "0.1+0.2, 1/3, 100/7, 1/7, 1.5, 2.0, 1e15, 1e16, 1e300, 1e-300, \
                       0.0000001234, 2/3*1e-20, 123456789.123456, -0.1-0.2";
    let values: Vec<f64> = vec![
        0.1 + 0.2,
        1.0 / 3.0,
        100.0 / 7.0,
        1.0 / 7.0,
        1.5,
        2.0,
        1e15,
        1e16,
        1e300,
        1e-300,
        0.0000001234,
        2.0 / 3.0 * 1e-20,
        123456789.123456,
        -0.1 - 0.2,
    ];

    let (lua, env) = engine();
    lua.eval_script(&format!(
        r#"
        local xs = {{ {expressions} }}
        local parts = {{}}
        for i, x in ipairs(xs) do parts[i] = tostring(x) end
        oslo.env.set("ZZ_TOSTRING", table.concat(parts, "|"))
        "#
    ))
    .expect("the VM renders them");

    let from_vm = param(&env, "ZZ_TOSTRING").expect("the VM set it");
    let from_value: Vec<String> = values
        .iter()
        .map(|x| Number::Float(*x).to_string())
        .collect();
    assert_eq!(
        from_vm,
        from_value.join("|"),
        "the VM and the shared formatter disagree"
    );
}

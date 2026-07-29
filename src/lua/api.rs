//! The `oslo.*` table: what a Lua program can ask the shell to do.
//!
//! Split from `engine.rs` so the engine owns the Lua state and this owns the surface a script
//! actually sees. Grouped by what each call acts on — commands, variables, the filesystem, the
//! shell itself — because that is how someone writing a script looks for them.
//!
//! The shape of every fallible call is Lua's own: `nil, message` on failure rather than an error,
//! so `local ok, err = oslo.cd(p)` reads the way `io.open` does. A raised error is reserved for a
//! caller mistake (a missing argument, a name that cannot exist), which is a bug in the script
//! rather than a condition it should handle.

use crate::env::Environment;
use crate::lua::engine::{BUILTIN_KEY_PREFIX, borrow_env, call_lua_builtin};
use mlua::prelude::*;
use std::sync::{Arc, Mutex};

/// Build the `oslo` table and install it as a global.
pub fn install(lua: &Lua, env: Arc<Mutex<Environment>>) -> LuaResult<()> {
    let oslo = lua.create_table()?;
    commands(lua, &oslo, &env)?;
    variables(lua, &oslo, &env)?;
    filesystem(lua, &oslo, &env)?;
    shell(lua, &oslo, &env)?;
    lua.globals().set("oslo", oslo)?;
    Ok(())
}

/// Running shell commands: `exec` for the side effect, `capture` for the output.
fn commands(lua: &Lua, oslo: &LuaTable, env: &Arc<Mutex<Environment>>) -> LuaResult<()> {
    // oslo.exec(cmd) -> status. Output goes wherever the shell's output goes.
    let env_exec = Arc::clone(env);
    oslo.set(
        "exec",
        lua.create_function(move |_, cmd: String| {
            let mut guard = borrow_env(&env_exec)?;
            let ast = crate::parser::parse_bash_script(&cmd).map_err(|e| e.into_lua_err())?;
            crate::exec::eval_command_list(&mut guard, &ast).map_err(|e| e.into_lua_err())
        })?,
    )?;

    // oslo.capture(cmd) -> { out = string, status = number }
    //
    // The gap that made Lua unusable for real work: `exec` reports only whether a command
    // succeeded, so anything that needed a command's *answer* — a version string, a device list,
    // the output of `uname` — had to be written in shell instead.
    //
    // A table rather than two return values: `r.out` says what it is at the call site, where
    // `local a, b = oslo.capture(...)` does not, and a table can grow a field later without
    // breaking every caller's positional unpacking.
    //
    // There is deliberately **no `err` field**. This runs the same capture `$(cmd)` does, which
    // takes stdout and leaves stderr attached to the shell's own — so an `err` here could only
    // ever be the empty string, and a field that is always empty reads as "the command printed
    // no diagnostics" rather than "nobody looked". A script that wants them folded together asks
    // for it the way a shell does: `oslo.capture("cmd 2>&1")`.
    //
    // Trailing newlines are stripped, matching `$(cmd)`: the shell's own capture does it, and a
    // Lua script comparing against "x" should not have to remember the command printed "x\n".
    let env_capture = Arc::clone(env);
    oslo.set(
        "capture",
        lua.create_function(move |lua, cmd: String| {
            let mut guard = borrow_env(&env_capture)?;
            let captured = crate::exec::eval_command_substitution(&mut guard, &cmd);
            // The substitution records its own child's status separately — `last_status` is the
            // shell's, which a capture does not touch, so reading that gave every command 0.
            let status = guard
                .take_substitution_status()
                .unwrap_or(guard.last_status);
            drop(guard);
            let result = lua.create_table()?;
            match captured {
                Ok(out) => {
                    result.set("out", out.trim_end_matches('\n'))?;
                    result.set("status", status)?;
                }
                Err(e) => {
                    result.set("out", "")?;
                    result.set("status", e.failure_status())?;
                }
            }
            Ok(result)
        })?,
    )?;
    Ok(())
}

/// Shell and environment variables.
fn variables(lua: &Lua, oslo: &LuaTable, env: &Arc<Mutex<Environment>>) -> LuaResult<()> {
    let env_get = Arc::clone(env);
    oslo.set(
        "get_var",
        lua.create_function(move |_, name: String| Ok(borrow_env(&env_get)?.get_param(&name)))?,
    )?;

    let env_set = Arc::clone(env);
    oslo.set(
        "set_var",
        lua.create_function(move |_, (name, value): (String, String)| {
            borrow_env(&env_set)?.set_var(&name, &value, true);
            Ok(())
        })?,
    )?;

    // oslo.unset(name) -> true. The other half of set_var; without it a script could create a
    // variable and never remove one.
    let env_unset = Arc::clone(env);
    oslo.set(
        "unset",
        lua.create_function(move |_, name: String| {
            borrow_env(&env_unset)?.unset_var(&name);
            Ok(true)
        })?,
    )?;

    // oslo.env() -> { NAME = value, ... }, the exported environment as one table.
    //
    // `get_var` answers one name at a time, which cannot express "what is set?" — a script could
    // not iterate the environment, filter it, or copy it. Exported names only: those are what a
    // child process would see, which is the question a script is usually asking.
    let env_all = Arc::clone(env);
    oslo.set(
        "env",
        lua.create_function(move |lua, ()| {
            let guard = borrow_env(&env_all)?;
            let table = lua.create_table()?;
            for (name, value) in guard.exported_vars() {
                table.set(name, value)?;
            }
            Ok(table)
        })?,
    )?;
    Ok(())
}

/// The working directory and pathname expansion.
fn filesystem(lua: &Lua, oslo: &LuaTable, env: &Arc<Mutex<Environment>>) -> LuaResult<()> {
    oslo.set(
        "get_pwd",
        lua.create_function(|_, ()| {
            Ok(std::env::current_dir()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string())
        })?,
    )?;

    // oslo.cd(path) -> true, or nil + message.
    //
    // Goes through the `cd` builtin rather than `std::env::set_current_dir` so that `$PWD`,
    // `$OLDPWD` and the directory stack agree with it afterwards — a Lua script that moved the
    // process without telling the shell would leave `pwd` reporting the old directory.
    let env_cd = Arc::clone(env);
    oslo.set(
        "cd",
        lua.create_function(move |_, path: String| {
            let mut guard = borrow_env(&env_cd)?;
            let args = vec!["cd".to_string(), path.clone()];
            match crate::env::builtins::builtin_cd(&mut guard, &args) {
                Ok(0) => Ok((Some(true), None)),
                Ok(_) => Ok((None, Some(format!("cd: {}: cannot change directory", path)))),
                Err(e) => Ok((None, Some(e.to_string()))),
            }
        })?,
    )?;

    // oslo.glob(pattern) -> { "a", "b", ... }, in the shell's own sorted order.
    //
    // Lua's standard library has no pathname expansion at all, so listing "every .conf in here"
    // meant shelling out. An empty table for no matches, never the pattern itself: returning the
    // unmatched pattern is a shell convention that has surprised people for forty years, and Lua
    // code checking `#matches == 0` is what a caller will naturally write.
    oslo.set(
        "glob",
        lua.create_function(|lua, pattern: String| {
            let table = lua.create_table()?;
            // One unquoted run, which is what makes every metacharacter in the string live —
            // the caller passed a pattern, not a word that might have been quoted.
            let field = [crate::expand::Run::new(
                pattern.clone(),
                crate::expand::Origin::Literal,
            )];
            let matches = crate::expand::glob::expand_glob(&field);
            // `expand_glob` yields the pattern back when nothing matched, the way an unquoted word
            // does in a command line. Here that would be a lie, so it becomes an empty table.
            let matches = if matches == vec![pattern.clone()] {
                Vec::new()
            } else {
                matches
            };
            for (i, path) in matches.into_iter().enumerate() {
                table.set(i + 1, path)?;
            }
            Ok(table)
        })?,
    )?;
    Ok(())
}

/// Aliases, builtins, the prompt, and the shell's exit status.
fn shell(lua: &Lua, oslo: &LuaTable, env: &Arc<Mutex<Environment>>) -> LuaResult<()> {
    let env_alias = Arc::clone(env);
    oslo.set(
        "set_alias",
        lua.create_function(move |_, (name, target): (String, String)| {
            borrow_env(&env_alias)?.set_alias(&name, &target);
            Ok(())
        })?,
    )?;

    let env_get_alias = Arc::clone(env);
    oslo.set(
        "get_alias",
        lua.create_function(move |_, name: String| {
            Ok(borrow_env(&env_get_alias)?
                .get_alias(&name)
                .map(str::to_string))
        })?,
    )?;

    // oslo.exit(code) -> never returns.
    //
    // A Lua program's own way to decide what the shell exits with. Without it the only way to
    // choose an exit status was `oslo.exec("exit 3")`, which is a shell command pretending to be
    // a language feature. `os.exit` would leave the shell's own cleanup — the EXIT trap, the
    // flush — unrun, so this goes through the shell's exit path instead.
    oslo.set(
        "exit",
        lua.create_function(|_, code: Option<i32>| -> LuaResult<()> {
            Err(crate::error::ShellError::Exit(code.unwrap_or(0)).into_lua_err())
        })?,
    )?;

    // oslo.register_builtin(name, callback)
    //
    // The callback is stored and run (PLAN R9.8). Until that round it was *dropped* and a stub
    // registered under the name instead, which is worse than doing nothing:
    // `oslo.register_builtin('ls', …)` made `ls /` print nothing and exit 0.
    let env_builtin = Arc::clone(env);
    oslo.set(
        "register_builtin",
        lua.create_function(move |lua, (name, func): (String, LuaFunction)| {
            let name = name.trim().to_string();
            if name.is_empty() {
                return Err(mlua::Error::runtime(
                    "oslo.register_builtin: the builtin name must not be empty",
                ));
            }
            // Registered with the interpreter first: if this fails there is no name in the
            // shell's registry pointing at a callback that is not there.
            lua.set_named_registry_value(&format!("{}{}", BUILTIN_KEY_PREFIX, name), func)?;
            let mut guard = borrow_env(&env_builtin)?;
            let key = name.clone();
            guard.register_dynamic_builtin(&name, move |_env, args| {
                Ok(call_lua_builtin(&key, args))
            });
            Ok(())
        })?,
    )?;

    oslo.set(
        "set_prompt",
        lua.create_function(|lua, func: LuaFunction| {
            lua.set_named_registry_value("oslo_prompt_fn", func)?;
            Ok(())
        })?,
    )?;

    // There is deliberately no `oslo.set_right_prompt` (PLAN R9.7). It was registered, and
    // `render_right_prompt` had no caller anywhere, so a script that set one saw nothing.
    // Drawing it means writing the string at `width - len` and restoring the cursor before
    // handing the line over — and rustyline repaints from the prompt to end-of-line on every
    // keystroke, erasing it again. Advertising an API that cannot work is worse than not having
    // one; if it comes back it comes back with a line editor that supports it.
    Ok(())
}

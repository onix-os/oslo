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
//!
//! Nothing here reimplements shell behaviour. `oslo.cd` runs the `cd` builtin and `oslo.glob`
//! calls the shell's own globber, so the two interfaces cannot drift apart.

use crate::env::Environment;
use crate::lua::engine::{BUILTIN_KEY_PREFIX, PROMPT_KEY, Registry, borrow_env, call_lua_builtin};
use crate::lua::eval::value::{Table, Value};
use crate::lua::eval::{Interp, LuaError};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

mod convert;
pub(crate) mod external;
mod fs;
mod json;
mod path;
mod proc;
pub(crate) mod prompt;
mod re;
mod run;
pub(crate) mod segment;
mod shell;
pub(crate) mod tools;

pub(crate) use shell::handlers as hook_handlers;
pub(crate) mod util;

use util::{put, text};

/// Build the `oslo` table and install it as a global.
pub fn install(interp: &Rc<Interp>, registry: &Registry, env: Arc<Mutex<Environment>>) {
    let mut oslo = Table::new();
    run::install(interp, &mut oslo, &env);
    oslo.set(Value::str("fs"), fs::build());
    let mut paths = path::build();
    if let Value::Table(table) = &mut paths {
        prompt::shorten(&mut table.borrow_mut());
    }
    oslo.set(Value::str("path"), paths);
    prompt::install(&mut oslo, registry);
    // The settings tables exist before the config runs, empty, so that
    // `oslo.completion.max_rows = 5` is an assignment rather than an attempt to index nil. Every
    // one of these is read back after the config by walking the table, so an empty one that the
    // user never touches is indistinguishable from an absent one — which is what makes leaving
    // them here free.
    for name in ["completion", "suggest", "history", "keys"] {
        oslo.set(
            Value::str(name),
            Value::Table(Rc::new(RefCell::new(Table::new()))),
        );
    }
    oslo.set(Value::str("json"), json::build());
    oslo.set(Value::str("re"), re::build());
    oslo.set(Value::str("proc"), proc::build_proc());
    oslo.set(Value::str("job"), proc::build_job());

    // The converters, plus `from_json` as an alias for `oslo.json.decode` — the same function
    // under the name the rest of the `from_*` family has.
    let converters = convert::build();
    if let (Value::Table(into), Value::Table(json)) = (&converters, &oslo.get(&Value::str("json")))
    {
        let decode = json.borrow().get(&Value::str("decode"));
        into.borrow_mut().set(Value::str("from_json"), decode);
    }
    if let Value::Table(into) = &converters {
        for (name, f) in into.borrow().pairs() {
            oslo.set(name, f);
        }
    }
    commands(&mut oslo, &env);
    variables(&mut oslo, &env);
    filesystem(&mut oslo, &env);
    shell(&mut oslo, registry, &env);
    shell::install(&mut oslo, registry, &env);
    interp.set_global("oslo", Value::table(oslo));
}

/// Running shell commands: `exec` for the side effect, `capture` for the output.
fn commands(oslo: &mut Table, env: &Arc<Mutex<Environment>>) {
    // oslo.exec(cmd) -> status. Output goes wherever the shell's output goes.
    let env_exec = Arc::clone(env);
    put(oslo, "exec", move |_, args| {
        let cmd = text(&args, 1, "oslo.exec")?;
        let mut guard = borrow_env(&env_exec)?;
        let ast = crate::parser::parse_bash_script(&cmd)
            .map_err(|e| LuaError::new(format!("oslo.exec: {e}")))?;
        let status = crate::exec::eval_command_list(&mut guard, &ast)
            .map_err(|e| LuaError::new(format!("oslo.exec: {e}")))?;
        Ok(vec![Value::int(status as i64)])
    });

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
    put(oslo, "capture", move |_, args| {
        let cmd = text(&args, 1, "oslo.capture")?;
        let mut guard = borrow_env(&env_capture)?;
        let captured = crate::exec::eval_command_substitution(&mut guard, &cmd);
        // The substitution records its own child's status separately — `last_status` is the
        // shell's, which a capture does not touch, so reading that gave every command 0.
        let status = guard
            .take_substitution_status()
            .unwrap_or(guard.last_status);
        drop(guard);

        let mut result = Table::new();
        match captured {
            Ok(out) => {
                result.set(Value::str("out"), Value::str(out.trim_end_matches('\n')));
                result.set(Value::str("status"), Value::int(status as i64));
            }
            Err(e) => {
                result.set(Value::str("out"), Value::str(""));
                result.set(Value::str("status"), Value::int(e.failure_status() as i64));
            }
        }
        Ok(vec![Value::table(result)])
    });
}

/// Shell and environment variables.
fn variables(oslo: &mut Table, env: &Arc<Mutex<Environment>>) {
    let env_get = Arc::clone(env);
    put(oslo, "get_var", move |_, args| {
        let name = text(&args, 1, "oslo.get_var")?;
        Ok(vec![match borrow_env(&env_get)?.get_param(&name) {
            Some(value) => Value::str(value),
            None => Value::Nil,
        }])
    });

    let env_set = Arc::clone(env);
    put(oslo, "set_var", move |_, args| {
        let name = text(&args, 1, "oslo.set_var")?;
        let value = text(&args, 2, "oslo.set_var")?;
        borrow_env(&env_set)?.set_var(&name, &value, true);
        Ok(Vec::new())
    });

    // oslo.unset(name) -> true. The other half of set_var; without it a script could create a
    // variable and never remove one.
    let env_unset = Arc::clone(env);
    put(oslo, "unset", move |_, args| {
        let name = text(&args, 1, "oslo.unset")?;
        borrow_env(&env_unset)?.unset_var(&name);
        Ok(vec![Value::Bool(true)])
    });

    // oslo.env() -> { NAME = value, ... }, the exported environment as one table.
    //
    // `get_var` answers one name at a time, which cannot express "what is set?" — a script could
    // not iterate the environment, filter it, or copy it. Exported names only: those are what a
    // child process would see, which is the question a script is usually asking.
    let env_all = Arc::clone(env);
    put(oslo, "env", move |_, _| {
        let guard = borrow_env(&env_all)?;
        let mut table = Table::new();
        for (name, value) in guard.exported_vars() {
            table.set(Value::str(name), Value::str(value));
        }
        Ok(vec![Value::table(table)])
    });
}

/// The working directory and pathname expansion.
fn filesystem(oslo: &mut Table, env: &Arc<Mutex<Environment>>) {
    put(oslo, "get_pwd", |_, _| {
        Ok(vec![Value::str(
            std::env::current_dir()
                .unwrap_or_default()
                .to_string_lossy(),
        )])
    });

    // oslo.cd(path) -> true, or nil + message.
    //
    // Goes through the `cd` builtin rather than `std::env::set_current_dir` so that `$PWD`,
    // `$OLDPWD` and the directory stack agree with it afterwards — a Lua script that moved the
    // process without telling the shell would leave `pwd` reporting the old directory.
    let env_cd = Arc::clone(env);
    put(oslo, "cd", move |_, args| {
        let path = text(&args, 1, "oslo.cd")?;
        let mut guard = borrow_env(&env_cd)?;
        let argv = vec!["cd".to_string(), path.clone()];
        Ok(match crate::env::builtins::builtin_cd(&mut guard, &argv) {
            Ok(0) => vec![Value::Bool(true)],
            Ok(_) => vec![
                Value::Nil,
                Value::str(format!("cd: {path}: cannot change directory")),
            ],
            Err(e) => vec![Value::Nil, Value::str(e.to_string())],
        })
    });

    // oslo.glob(pattern) -> { "a", "b", ... }, in the shell's own sorted order.
    //
    // Lua's standard library has no pathname expansion at all, so listing "every .conf in here"
    // meant shelling out. An empty table for no matches, never the pattern itself: returning the
    // unmatched pattern is a shell convention that has surprised people for forty years, and Lua
    // code checking `#matches == 0` is what a caller will naturally write.
    put(oslo, "glob", |_, args| {
        let pattern = text(&args, 1, "oslo.glob")?;
        // One unquoted run, which is what makes every metacharacter in the string live — the
        // caller passed a pattern, not a word that might have been quoted.
        let field = [crate::expand::Run::new(
            pattern.clone(),
            crate::expand::Origin::Literal,
        )];
        let matches = crate::expand::glob::expand_glob(&field);
        // `expand_glob` yields the pattern back when nothing matched, the way an unquoted word
        // does in a command line. Here that would be a lie, so it becomes an empty table.
        let matches = if matches == vec![pattern] {
            Vec::new()
        } else {
            matches
        };
        let mut table = Table::new();
        for (i, path) in matches.into_iter().enumerate() {
            table.set(Value::int(i as i64 + 1), Value::str(path));
        }
        Ok(vec![Value::table(table)])
    });
}

/// Aliases, builtins, the prompt, and the shell's exit status.
fn shell(oslo: &mut Table, registry: &Registry, env: &Arc<Mutex<Environment>>) {
    let env_alias = Arc::clone(env);
    put(oslo, "set_alias", move |_, args| {
        let name = text(&args, 1, "oslo.set_alias")?;
        let target = text(&args, 2, "oslo.set_alias")?;
        borrow_env(&env_alias)?.set_alias(&name, &target);
        Ok(Vec::new())
    });

    let env_get_alias = Arc::clone(env);
    put(oslo, "get_alias", move |_, args| {
        let name = text(&args, 1, "oslo.get_alias")?;
        Ok(vec![match borrow_env(&env_get_alias)?.get_alias(&name) {
            Some(target) => Value::str(target),
            None => Value::Nil,
        }])
    });

    // oslo.exit(code) -> never returns.
    //
    // A Lua program's own way to decide what the shell exits with. Without it the only way to
    // choose an exit status was `oslo.exec("exit 3")`, which is a shell command pretending to be
    // a language feature. `os.exit` would leave the shell's own cleanup — the EXIT trap, the
    // flush — unrun, so this goes through the shell's exit path instead.
    put(oslo, "exit", |_, args| {
        let code = match args.first() {
            Some(v) => v.as_number().map(|n| n.as_float() as i32).unwrap_or(0),
            None => 0,
        };
        Err(LuaError::exit_request(code))
    });

    // oslo.register_builtin(name, callback)
    //
    // The callback is stored and run (PLAN R9.8). Until that round it was *dropped* and a stub
    // registered under the name instead, which is worse than doing nothing:
    // `oslo.register_builtin('ls', …)` made `ls /` print nothing and exit 0.
    let env_builtin = Arc::clone(env);
    let registry_builtin = Rc::clone(registry);
    put(oslo, "register_builtin", move |_, args| {
        let name = text(&args, 1, "oslo.register_builtin")?.trim().to_string();
        if name.is_empty() {
            return Err(LuaError::new(
                "oslo.register_builtin: the builtin name must not be empty",
            ));
        }
        let Some(callback @ Value::Function(_)) = args.get(1) else {
            return Err(LuaError::new(
                "oslo.register_builtin: the second argument must be a function",
            ));
        };
        // Stored first: if the shell registration fails there is no name in the registry
        // pointing at a callback that is not there.
        registry_builtin
            .borrow_mut()
            .insert(format!("{BUILTIN_KEY_PREFIX}{name}"), callback.clone());

        let mut guard = borrow_env(&env_builtin)?;
        let key = name.clone();
        guard.register_dynamic_builtin(&name, move |_env, args| Ok(call_lua_builtin(&key, args)));
        Ok(Vec::new())
    });

    let registry_prompt = Rc::clone(registry);
    put(oslo, "set_prompt", move |_, args| {
        let Some(f @ Value::Function(_)) = args.first() else {
            return Err(LuaError::new(
                "oslo.set_prompt: the argument must be a function",
            ));
        };
        registry_prompt
            .borrow_mut()
            .insert(PROMPT_KEY.to_string(), f.clone());
        Ok(Vec::new())
    });

    // There is deliberately no `oslo.set_right_prompt` (PLAN R9.7). It was registered, and
    // `render_right_prompt` had no caller anywhere, so a script that set one saw nothing.
    // Drawing it means writing the string at `width - len` and restoring the cursor before
    // handing the line over — and rustyline repaints from the prompt to end-of-line on every
    // keystroke, erasing it again. Advertising an API that cannot work is worse than not having
    // one; if it comes back it comes back with a line editor that supports it.
}

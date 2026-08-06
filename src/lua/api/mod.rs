//! The `oslo.*` table: what a Lua program can ask the shell to do.
//!
//! Split from `engine.rs` so the engine owns the Lua state and this owns the surface a script
//! actually sees. Grouped by what each call acts on — commands, variables, the filesystem, the
//! shell itself — because that is how someone writing a script looks for them.
//!
//! The shape of every fallible call is Lua's own: `nil, message` on failure rather than an error,
//! so `local ok, err = oslo.sys.cd(p)` reads the way `io.open` does. A raised error is reserved for a
//! caller mistake (a missing argument, a name that cannot exist), which is a bug in the script
//! rather than a condition it should handle.
//!
//! Nothing here reimplements shell behaviour. `oslo.sys.cd` runs the `cd` builtin and `oslo.glob`
//! calls the shell's own globber, so the two interfaces cannot drift apart.

use crate::env::Environment;
use crate::lua::engine::{BUILTIN_KEY_PREFIX, PROMPT_KEY, Registry, borrow_env, call_lua_builtin};
use crate::lua::eval::value::{Table, Value};
use crate::lua::eval::{Interp, LuaError};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

mod convert;
mod direnv;
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
pub(crate) mod tool;
pub(crate) mod tools;
mod ui;

pub mod hooks;
pub(crate) use shell::handlers as hook_handlers;
pub(crate) mod util;

use util::{native, put, text};

/// Build the `oslo` table and install it as a global.
pub fn install(interp: &Rc<Interp>, registry: &Registry, env: Arc<Mutex<Environment>>) {
    let mut oslo = Table::new();
    // **Four namespaces, declared before anything fills them.** `oslo` had grown a flat surface of
    // twenty-two names where `nix_develop` sat between `login` and `path_add`, which tells a reader
    // nothing about what belongs with what. `fs`, `json`, `path`, `proc` and `re` were already
    // tables; these are the rest, grouped the same way. They are created here rather than at the
    // point of use because five different builders contribute to them.
    let mut variables_t = Table::new();
    let mut process = Table::new();
    let mut system = Table::new();
    let mut ui = Table::new();
    run::install(interp, &mut oslo, &env);
    oslo.set(Value::str("fs"), fs::build());
    let mut paths = path::build();
    if let Value::Table(table) = &mut paths {
        prompt::shorten(&mut table.borrow_mut());
    }
    oslo.set(Value::str("path"), paths);
    prompt::install(&mut oslo, &mut ui, registry);
    tool::install(&mut oslo);
    // The settings tables exist before the config runs, empty, so that
    // `oslo.completion.max_rows = 5` is an assignment rather than an attempt to index nil. Every
    // one of these is read back after the config by walking the table, so an empty one that the
    // user never touches is indistinguishable from an absent one — which is what makes leaving
    // them here free.
    //
    // **Every settings table, not most of them.** `vi`, `notify` and `dirs` were missing, so
    // `oslo.vi.enabled = false` — the spelling the README documents — died with "attempt to index
    // a nil value" while `oslo.vi = { enabled = false }` worked. A config language where two
    // spellings of the same thing differ by whether somebody remembered to add a line here is one
    // nobody can hold in their head.
    //
    // **A fallback, never a replacement.** This used to assign unconditionally, and `theme` is in
    // the list, so it overwrote the real `oslo.theme` that `prompt::install` had just built one
    // line above — taking `oslo.theme.styles` and its `__newindex` with it. The README's
    // `oslo.theme.styles["my.dir"] = …` therefore died on an empty table, which is the very
    // failure this loop exists to prevent, caused by the fix for it. A namespace that something
    // else already built with behaviour keeps what it has.
    for name in [
        "completion",
        "suggest",
        "history",
        "keys",
        "finder",
        "misc",
        "vi",
        "notify",
        "dirs",
        "theme",
        "abbr",
    ] {
        if matches!(oslo.get(&Value::str(name)), Value::Nil) {
            oslo.set(
                Value::str(name),
                Value::Table(Rc::new(RefCell::new(Table::new()))),
            );
        }
    }

    // `oslo.builtin` is the same thing one level deeper: it is a namespace *per builtin*, so
    // `oslo.builtin.rm.to_tmp = true` indexes twice and both tables have to be here. A new
    // builtin with settings adds a line beside `rm`.
    let mut builtin = Table::new();
    builtin.set(
        Value::str("rm"),
        Value::Table(Rc::new(RefCell::new(Table::new()))),
    );
    oslo.set(
        Value::str("builtin"),
        Value::Table(Rc::new(RefCell::new(builtin))),
    );
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
    // **Four namespaces, not twenty-two loose names.** `oslo` had grown a flat surface where
    // `nix_develop` sat between `login` and `path_add`, which tells a reader nothing about what
    // belongs with what. `fs`, `json`, `path`, `proc` and `re` were already tables; these are the
    // rest of the surface grouped the same way. What stays on `oslo` itself is only what belongs
    // to no group: `glob`, and the two `register_*` hooks that extend the shell.
    ui::install(&mut ui);
    commands(&mut process, &env);
    variables(&mut variables_t, &env);
    filesystem(&mut oslo, &mut system, &env);
    shell(
        &mut oslo,
        &mut process,
        &mut ui,
        &mut variables_t,
        registry,
        &env,
    );

    shell::install(&mut oslo, &mut system, &mut process, registry, &env);

    oslo.set(Value::str("env"), Value::table(variables_t));
    extend(&mut oslo, "proc", process);
    oslo.set(Value::str("sys"), Value::table(system));
    oslo.set(Value::str("ui"), Value::table(ui));
    oslo.set(Value::str("direnv"), direnv::build(&env));
    let oslo = Value::table(oslo);
    publish(interp, &oslo);
    interp.set_global("oslo", oslo);
}

/// Make every `oslo.X` table `require`-able as `oslo.X`.
///
/// **This is what makes them libraries rather than fields on a global.** A namespace you can only
/// reach by indexing a global is a naming convention; a module you can `require` is a thing with a
/// name, that can be aliased locally, shadowed by one of your own in `~/.config/oslo/lua`, and
/// depended on by a library that never mentions `oslo` at all:
///
/// ```lua
/// local ui = require "oslo.ui"
/// local env = require "oslo.env"
/// ```
///
/// Registered in `package.preload` rather than on disk, so the lookup never touches the filesystem
/// and a user's own `oslo/ui.lua` cannot shadow the built-in by accident — `preload` wins.
fn publish(interp: &Rc<Interp>, oslo: &Value) {
    let Value::Table(table) = oslo else { return };
    let Value::Table(package) = interp.global("package") else {
        return;
    };
    let Value::Table(preload) = package.borrow().get(&Value::str("preload")) else {
        return;
    };
    // Only the tables. A function on `oslo` is not a module, and registering one would make
    // `require "oslo.glob"` answer with something that is not what `require` promises.
    let entries: Vec<(String, Value)> = table
        .borrow()
        .pairs()
        .into_iter()
        .filter_map(|(name, value)| match (&name, &value) {
            (Value::Str(name), Value::Table(_)) => Some((name.to_string(), value.clone())),
            _ => None,
        })
        .collect();
    for (name, value) in entries {
        let module = value.clone();
        preload.borrow_mut().set(
            Value::str(format!("oslo.{name}")),
            native("preload", move |_, _| Ok(vec![module.clone()])),
        );
    }
    // `require "oslo"` answers with the whole thing, so a script can take the lot in one line.
    let whole = oslo.clone();
    preload.borrow_mut().set(
        Value::str("oslo"),
        native("preload", move |_, _| Ok(vec![whole.clone()])),
    );
}

/// Fold everything in `additions` into the table already on `oslo` under `name`.
///
/// `proc` is built by its own module and then extended here, because running a command and
/// signalling one are the same subject arriving from two places. Merged rather than replaced: the
/// module's own entries must survive.
fn extend(oslo: &mut Table, name: &str, additions: Table) {
    match oslo.get(&Value::str(name)) {
        Value::Table(existing) => {
            for (key, value) in additions.pairs() {
                existing.borrow_mut().set(key, value);
            }
        }
        _ => oslo.set(Value::str(name), Value::table(additions)),
    }
}

/// Running shell commands: `exec` for the side effect, `capture` for the output.
fn commands(oslo: &mut Table, env: &Arc<Mutex<Environment>>) {
    // oslo.proc.exec(cmd) -> status. Output goes wherever the shell's output goes.
    let env_exec = Arc::clone(env);
    put(oslo, "exec", move |_, args| {
        let cmd = text(&args, 1, "oslo.proc.exec")?;
        let mut guard = borrow_env(&env_exec)?;
        let ast = crate::parser::parse_bash_script(&cmd)
            .map_err(|e| LuaError::new(format!("oslo.proc.exec: {e}")))?;
        let status = crate::exec::eval_command_list(&mut guard, &ast)
            .map_err(|e| LuaError::new(format!("oslo.proc.exec: {e}")))?;
        Ok(vec![Value::int(status as i64)])
    });

    // oslo.proc.capture(cmd) -> { out = string, status = number }
    //
    // The gap that made Lua unusable for real work: `exec` reports only whether a command
    // succeeded, so anything that needed a command's *answer* — a version string, a device list,
    // the output of `uname` — had to be written in shell instead.
    //
    // A table rather than two return values: `r.out` says what it is at the call site, where
    // `local a, b = oslo.proc.capture(...)` does not, and a table can grow a field later without
    // breaking every caller's positional unpacking.
    //
    // There is deliberately **no `err` field**. This runs the same capture `$(cmd)` does, which
    // takes stdout and leaves stderr attached to the shell's own — so an `err` here could only
    // ever be the empty string, and a field that is always empty reads as "the command printed
    // no diagnostics" rather than "nobody looked". A script that wants them folded together asks
    // for it the way a shell does: `oslo.proc.capture("cmd 2>&1")`.
    //
    // Trailing newlines are stripped, matching `$(cmd)`: the shell's own capture does it, and a
    // Lua script comparing against "x" should not have to remember the command printed "x\n".
    let env_capture = Arc::clone(env);
    put(oslo, "capture", move |_, args| {
        let cmd = text(&args, 1, "oslo.proc.capture")?;
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
    put(oslo, "get", move |_, args| {
        let name = text(&args, 1, "oslo.env.get")?;
        Ok(vec![match borrow_env(&env_get)?.get_param(&name) {
            Some(value) => Value::str(value),
            None => Value::Nil,
        }])
    });

    let env_set = Arc::clone(env);
    put(oslo, "set", move |_, args| {
        let name = text(&args, 1, "oslo.env.set")?;
        let value = text(&args, 2, "oslo.env.set")?;
        borrow_env(&env_set)?.set_var(&name, &value, true);
        Ok(Vec::new())
    });

    // oslo.env.unset(name) -> true. The other half of set_var; without it a script could create a
    // variable and never remove one.
    let env_unset = Arc::clone(env);
    put(oslo, "unset", move |_, args| {
        let name = text(&args, 1, "oslo.env.unset")?;
        borrow_env(&env_unset)?.unset_var(&name);
        Ok(vec![Value::Bool(true)])
    });

    // oslo.env.all() -> { NAME = value, ... }, the exported environment as one table.
    //
    // `get_var` answers one name at a time, which cannot express "what is set?" — a script could
    // not iterate the environment, filter it, or copy it. Exported names only: those are what a
    // child process would see, which is the question a script is usually asking.
    let env_all = Arc::clone(env);
    put(oslo, "all", move |_, _| {
        let guard = borrow_env(&env_all)?;
        let mut table = Table::new();
        for (name, value) in guard.exported_vars() {
            table.set(Value::str(name), Value::str(value));
        }
        Ok(vec![Value::table(table)])
    });
}

/// The working directory and pathname expansion.
fn filesystem(oslo: &mut Table, system: &mut Table, env: &Arc<Mutex<Environment>>) {
    put(system, "pwd", |_, _| {
        Ok(vec![Value::str(
            std::env::current_dir()
                .unwrap_or_default()
                .to_string_lossy(),
        )])
    });

    // oslo.sys.cd(path) -> true, or nil + message.
    //
    // Goes through the `cd` builtin rather than `std::env::set_current_dir` so that `$PWD`,
    // `$OLDPWD` and the directory stack agree with it afterwards — a Lua script that moved the
    // process without telling the shell would leave `pwd` reporting the old directory.
    let env_cd = Arc::clone(env);
    put(system, "cd", move |_, args| {
        let path = text(&args, 1, "oslo.sys.cd")?;
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
fn shell(
    oslo: &mut Table,
    process: &mut Table,
    ui: &mut Table,
    names: &mut Table,
    registry: &Registry,
    env: &Arc<Mutex<Environment>>,
) {
    // `oslo.source(path)` — run a **shell** file in this shell.
    //
    // The config is Lua, but a person's `~/.profile` and their pile of `aliases.sh` are shell and
    // are shared with every other shell they use. Rewriting them in Lua is not an answer; being
    // able to read them is.
    //
    // Sourced, not run: `oslo.run{"sh", path}` would execute the file in a *child*, where every
    // alias, export and function it defines dies with the process. This is the same code path as
    // the `source` builtin, so it lands in the environment the prompt actually uses.
    //
    // Answers `true`, or `false` and a message — a missing `~/.profile` is the ordinary case on a
    // fresh machine and must not stop the rest of the config from loading.
    let env_source = Arc::clone(env);
    put(oslo, "source", move |_, args| {
        let path = text(&args, 1, "oslo.source")?;
        let mut guard = borrow_env(&env_source)?;
        match crate::env::builtins::builtin_source(
            &mut guard,
            &["source".to_string(), path.clone()],
        ) {
            Ok(0) => Ok(vec![Value::Bool(true)]),
            Ok(status) => Ok(vec![
                Value::Bool(false),
                Value::str(format!("{path}: exited {status}")),
            ]),
            Err(e) => Ok(vec![Value::Bool(false), Value::str(e.to_string())]),
        }
    });

    let env_alias = Arc::clone(env);
    put(names, "set_alias", move |_, args| {
        let name = text(&args, 1, "oslo.env.set_alias")?;
        let target = text(&args, 2, "oslo.env.set_alias")?;
        borrow_env(&env_alias)?.set_alias(&name, &target);
        Ok(Vec::new())
    });

    let env_get_alias = Arc::clone(env);
    put(names, "alias", move |_, args| {
        let name = text(&args, 1, "oslo.env.alias")?;
        Ok(vec![match borrow_env(&env_get_alias)?.get_alias(&name) {
            Some(target) => Value::str(target),
            None => Value::Nil,
        }])
    });

    // oslo.proc.exit(code) -> never returns.
    //
    // A Lua program's own way to decide what the shell exits with. Without it the only way to
    // choose an exit status was `oslo.proc.exec("exit 3")`, which is a shell command pretending to be
    // a language feature. `os.exit` would leave the shell's own cleanup — the EXIT trap, the
    // flush — unrun, so this goes through the shell's exit path instead.
    put(process, "exit", |_, args| {
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
    put(ui, "prompt", move |_, args| {
        let Some(f @ Value::Function(_)) = args.first() else {
            return Err(LuaError::new(
                "oslo.ui.prompt: the argument must be a function",
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

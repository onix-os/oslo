//! What a script can ask *about* the shell, and what it can ask the shell to call back into.
//!
//! Two groups that belong together because both are about the session rather than about doing
//! something: introspection (`oslo.sys.interactive`, `oslo.sys.user`, `oslo.proc.status`) and hooks
//! (`oslo.on.precmd`).
//!
//! **Hooks are named setters, not an event bus.** `oslo.on.precmd(fn)` returns a handle and
//! `handle:remove()` takes it off again. A general `on("precmd", fn)` bus reads better right up
//! to the point where you want to remove a handler: Hilbish's `bait` needs the *identical*
//! function reference back, so any handler written inline — which is nearly all of them — can
//! never be removed at all.

use super::util::{list, native, ok, put, record, text};
use crate::env::Environment;
use crate::lua::engine::{Registry, borrow_env};
use crate::lua::eval::value::{Table, Value};
use crate::lua::eval::{LuaError, Value as V};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

/// Registry key prefix under which a hook's handlers live.
pub(crate) const HOOK_PREFIX: &str = "hook:";

/// The hooks a script may attach to, and what each one is handed.
///
/// A fixed list rather than an open set: a name that is never fired is indistinguishable from a
/// typo, and `oslo.on.precmb(fn)` silently doing nothing for ever is the failure mode this
/// avoids.
pub(crate) const HOOKS: [&str; 8] = [
    // **`preexec` and `precmd` are the names the rest of the world uses**, and oslo had them
    // crossed: what it called `precmd` fires *before the command*, which is preexec everywhere
    // else, and there was no hook at all for "a prompt is about to be drawn" — the thing every
    // prompt integration actually installs.
    //
    // Both spellings now exist and fire together, so no config breaks; `preexec` is the one to
    // write. `prompt` is the missing one.
    "preexec",
    "precmd",
    "postexec",
    "postcmd",
    "prompt",
    "cd",
    "command-not-found",
    // Every keystroke, before the editor acts on it. See `KEY_WATCHED`.
    "key",
];

/// The `key` hook's name, spelled once.
pub(crate) const KEY: &str = "key";

/// Whether anything has ever attached to the `key` hook.
///
/// **This exists so that not using the hook costs nothing.** Every other hook fires at a moment
/// that already involves running a command; this one fires on every keypress, and asking the
/// registry each time — a string format, a map lookup, a `Vec` — would put that on the path of
/// simply typing. One relaxed load answers it instead.
///
/// Never cleared. Removing the last handler leaves the flag set and the lookup then finds an empty
/// list, which costs a little on a session that attached and detached; clearing it correctly would
/// mean counting handlers across reloads, and a wrong count here silently kills the hook.
static KEY_WATCHED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Whether the `key` hook is worth asking about. See [`KEY_WATCHED`].
pub fn key_hook_watched() -> bool {
    KEY_WATCHED.load(std::sync::atomic::Ordering::Relaxed)
}

/// Add the introspection fields, `oslo.opts` and `oslo.on` to the `oslo` table.
pub fn install(
    oslo: &mut Table,
    system: &mut Table,
    process: &mut Table,
    registry: &Registry,
    env: &Arc<Mutex<Environment>>,
) {
    facts(oslo, system, process, env);
    oslo.set(Value::str("on"), hooks(registry));
}

/// What the shell knows about itself, split by subject rather than left on `oslo`.
///
/// Who and where you are is `oslo.sys`; which process this is and how the last one ended is
/// `oslo.proc`. `oslo.version` stays on `oslo` itself — it describes the whole thing.
fn facts(oslo: &mut Table, system: &mut Table, process: &mut Table, env: &Arc<Mutex<Environment>>) {
    oslo.set(Value::str("version"), Value::str(env!("CARGO_PKG_VERSION")));

    // Read at call time, not at startup: a script that changes `$USER` or `hostname` mid-session
    // should see what it changed, and a value frozen at startup is a lie that is hard to spot.
    let env_user = Arc::clone(env);
    put(system, "user", move |_, _| {
        let guard = borrow_env(&env_user)?;
        ok(
            match guard.get_var("USER").or_else(|| guard.get_var("LOGNAME")) {
                Some(name) => Value::str(name),
                None => Value::Nil,
            },
        )
    });

    put(system, "host", |_, _| {
        ok(match nix::unistd::gethostname() {
            Ok(name) => Value::str(name.to_string_lossy()),
            Err(_) => Value::Nil,
        })
    });

    let env_interactive = Arc::clone(env);
    put(system, "interactive", move |_, _| {
        let guard = borrow_env(&env_interactive)?;
        ok(Value::Bool(
            guard
                .options()
                .is_set(crate::env::options::ShellOption::Interactive),
        ))
    });

    let env_login = Arc::clone(env);
    put(system, "login", move |_, _| {
        let guard = borrow_env(&env_login)?;
        // A login shell is one invoked with a `-` in front of its name, which is what `$0` keeps.
        ok(Value::Bool(guard.shell_name.starts_with('-')))
    });

    // oslo.proc.status() -> $? — the status of the last command, whichever language ran it.
    let env_status = Arc::clone(env);
    put(process, "status", move |_, _| {
        ok(Value::int(borrow_env(&env_status)?.last_status as i64))
    });

    // oslo.proc.pid() and oslo.proc.ppid(), which a script needs to name itself in a lock file or a log.
    put(process, "pid", |_, _| {
        ok(Value::int(std::process::id() as i64))
    });
    put(process, "ppid", |_, _| {
        ok(Value::int(nix::unistd::getppid().as_raw() as i64))
    });

    // oslo.opts — the knobs the config needs, as a plain table.
    //
    // A table rather than getters because a config file's natural shape is assignment, and
    // because the values are read by the *shell* rather than by Lua: they live in shell variables
    // so that `$OSLO_DEFAULT_MODE` set from either language means the same thing.
    let env_opts = Arc::clone(env);
    let mut opts = Table::new();
    let env_set = Arc::clone(&env_opts);
    put(&mut opts, "set", move |_, args| {
        let name = text(&args, 1, "oslo.opts.set")?;
        let value = text(&args, 2, "oslo.opts.set")?;
        // Namespaced on the way in, so `oslo.opts.set("default_mode", …)` is `$OSLO_DEFAULT_MODE`
        // and a script cannot reach an unrelated variable through it.
        borrow_env(&env_set)?.set_var(&option_var(&name), &value, false);
        ok(Value::Bool(true))
    });
    put(&mut opts, "get", move |_, args| {
        let name = text(&args, 1, "oslo.opts.get")?;
        ok(match borrow_env(&env_opts)?.get_var(&option_var(&name)) {
            Some(value) => Value::str(value),
            None => Value::Nil,
        })
    });
    // `toggle_key` was here. It is now `oslo.keys`, which is where every other key binding already
    // lived — see `crate::startup::mode::TOGGLE_KEY`.
    put(&mut opts, "names", |_, _| {
        ok(list(["default_mode"].into_iter().map(Value::str)))
    });
    oslo.set(Value::str("opts"), Value::table(opts));
}

/// The shell variable an option name maps to.
fn option_var(name: &str) -> String {
    format!("OSLO_{}", name.trim().to_ascii_uppercase())
}

/// `oslo.on` — one setter per hook, each returning a handle that can remove itself.
fn hooks(registry: &Registry) -> Value {
    let mut on = Table::new();
    for name in HOOKS {
        let registry = Rc::clone(registry);
        let key = format!("{HOOK_PREFIX}{name}");
        put(&mut on, name, move |_, args| {
            let Some(handler @ Value::Function(_)) = args.first() else {
                return Err(LuaError::new(format!(
                    "oslo.on.{name}: the argument must be a function"
                )));
            };
            if name == KEY {
                KEY_WATCHED.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            let id = append(&registry, &key, handler.clone());
            ok(handle(&registry, &key, id))
        });
    }
    Value::table(on)
}

/// Add a handler to a hook's list, answering its position.
fn append(registry: &Registry, key: &str, handler: Value) -> i64 {
    let mut slots = registry.borrow_mut();
    let entry = slots.entry(key.to_string()).or_insert_with(|| {
        let table = Table::new();
        Value::table(table)
    });
    let Value::Table(list) = entry else {
        return 0;
    };
    let next = list.borrow().length() + 1;
    list.borrow_mut().set(Value::int(next), handler);
    next
}

/// The handle a hook setter returns: `handle:remove()` and nothing else.
///
/// A handle rather than the function itself, so a handler written inline can still be taken off
/// again — which is the whole reason this is not a `on(name, fn)` bus.
fn handle(registry: &Registry, key: &str, id: i64) -> Value {
    let registry = Rc::clone(registry);
    let key = key.to_string();
    record(vec![(
        "remove",
        native("hook handle", move |_, _| {
            if let Some(Value::Table(list)) = registry.borrow().get(&key) {
                // Replaced with `false` rather than removed, so every other handle's position
                // stays valid — shifting the list would silently re-point them at their
                // neighbours.
                list.borrow_mut().set(Value::int(id), Value::Bool(false));
            }
            ok(Value::Bool(true))
        }),
    )])
}

/// Every handler currently attached to `name`, in the order they were added.
pub(crate) fn handlers(registry: &Registry, name: &str) -> Vec<Value> {
    let key = format!("{HOOK_PREFIX}{name}");
    let Some(V::Table(list)) = registry.borrow().get(&key).cloned() else {
        return Vec::new();
    };
    // Removed handlers are `false` in the list rather than gaps, so they are skipped here.
    let attached = list.borrow();
    attached
        .sequence()
        .iter()
        .filter(|v| matches!(v, V::Function(_)))
        .cloned()
        .collect()
}

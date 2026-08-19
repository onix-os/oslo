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
use crate::lua::engine::{Registry, borrow_env};
use oslo_base::value::{LuaError, Value as V};
use oslo_base::value::{Table, Value};
use oslo_shell::env::Environment;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

/// Registry key prefix under which a hook's handlers live.
pub(crate) const HOOK_PREFIX: &str = "hook:";

/// The moments a config may attach to, and the spellings that reach each one.
///
/// Moved to [`super::hooks`] when there were twenty of them rather than eight, and when the older
/// names became aliases of the `pre-`/`post-`/`on-` scheme rather than separate entries.
pub(crate) use super::hooks;

/// Add the introspection fields, `oslo.opts` and `oslo.on` to the `oslo` table.
pub fn install(
    oslo: &mut Table,
    system: &mut Table,
    process: &mut Table,
    registry: &Registry,
    env: &Arc<Mutex<Environment>>,
) {
    facts(oslo, system, process, env);
    oslo.set_str("on", hooks(registry));
}

/// What the shell knows about itself, split by subject rather than left on `oslo`.
///
/// Who and where you are is `oslo.sys`; which process this is and how the last one ended is
/// `oslo.proc`. `oslo.version` stays on `oslo` itself — it describes the whole thing.
fn facts(oslo: &mut Table, system: &mut Table, process: &mut Table, env: &Arc<Mutex<Environment>>) {
    oslo.set(
        Value::str("version"),
        Value::str(oslo_base::version::current()),
    );

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
        ok(Value::Bool(guard.options().is_set(
            oslo_shell::env::options::ShellOption::Interactive,
        )))
    });

    let env_login = Arc::clone(env);
    put(system, "login", move |_, _| {
        let guard = borrow_env(&env_login)?;
        // A login shell is one invoked with a `-` in front of its name, which is what `$0` keeps.
        ok(Value::Bool(guard.shell_name.starts_with('-')))
    });

    // oslo.sys.terminal() — what the terminal answered when oslo asked, before the first prompt.
    //
    // **Negotiated, not guessed from `$TERM`.** oslo sends `CSI ? u` and a Primary DA barrier at
    // startup and reads what comes back, so these are the terminal's own answers. They decide real
    // behaviour — `kitty_keyboard` is why Ctrl+Enter and Ctrl+Tab either exist or do not — and
    // until this existed there was no way to see which side of that a session had landed on. That
    // turned every "this key does nothing" into a guess.
    put(system, "terminal", |_, _| {
        let mut out = Table::new();
        match oslo_ui::term::capability::snapshot_if_initialized() {
            Some(caps) => {
                out.set_str("kitty_keyboard", Value::Bool(caps.kitty_keyboard));
                out.set_str("synchronized_output", Value::Bool(caps.synchronized_output));
                out.set_str("bracketed_paste", Value::Bool(caps.bracketed_paste));
                out.set_str("semantic_clicks", Value::Bool(caps.semantic_clicks));
                out.set_str("legacy_clicks", Value::Bool(caps.legacy_clicks));
            }
            // Nothing was negotiated: a script, a pipe, `$TERM=dumb`. Reported as absent rather
            // than as false, because "not asked" and "asked and told no" are different facts.
            None => out.set_str("negotiated", Value::Bool(false)),
        }
        ok(Value::table(out))
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
    oslo.set_str("opts", Value::table(opts));
}

/// The shell variable an option name maps to.
fn option_var(name: &str) -> String {
    format!("OSLO_{}", name.trim().to_ascii_uppercase())
}

/// `oslo.on` — one setter per spelling, each returning a handle that can remove itself.
///
/// Every spelling of a moment is installed, and all of them key on the *canonical* name — so
/// `oslo.on.preexec(f)` and `oslo.on["pre-cmd"](g)` build one list and fire together.
fn hooks(registry: &Registry) -> Value {
    let mut on = Table::new();
    for &(spelling, index) in hooks::spellings() {
        let registry = Rc::clone(registry);
        let canonical = hooks::HOOKS[index].name;
        let key = format!("{HOOK_PREFIX}{canonical}");
        let reported = spelling;
        put(&mut on, spelling, move |_, args| {
            let Some(handler @ Value::Function(_)) = args.first() else {
                return Err(LuaError::new(format!(
                    "oslo.on.{reported}: the argument must be a function"
                )));
            };
            // Recorded before the handler is stored, so a hot-path check can never see the
            // handler without the bit that says to look for it.
            hooks::attached(index);
            let id = append(&registry, &key, handler.clone());
            ok(handle(&registry, &key, id))
        });
    }
    user_events(&mut on, registry);
    Value::table(on)
}

/// Where a *plugin* names the moment.
///
/// ```lua
/// oslo.on.user("notes:saved", function(data, name) … end)   -- anyone who cares
/// oslo.on.emit("notes:saved", { key = k })                  -- the plugin that did it
/// ```
///
/// **The one thing two plugins can share.** Every hook above is a name oslo chose, so two plugins
/// could previously compose only through the filesystem; this is the same mechanism neovim gets from
/// its single `User` event, which is what its ecosystem coordinates through.
///
/// The storage is the built-in hooks' own, under a `user:` prefix — so a handler is removed by the
/// same handle, fired in the order it was attached, and a raising one is reported while the rest
/// still run. What is *not* shared is the watched bitset: that exists so an unused hot-path hook
/// costs nothing, and nothing here is on a hot path.
fn user_events(on: &mut Table, registry: &Registry) {
    let for_attach = Rc::clone(registry);
    put(on, "user", move |_, args| {
        let name = event_name(args.first(), "oslo.on.user")?;
        let Some(handler @ Value::Function(_)) = args.get(1) else {
            return Err(LuaError::new(
                "oslo.on.user: the second argument must be a function".to_string(),
            ));
        };
        let key = format!("{HOOK_PREFIX}user:{name}");
        let id = append(&for_attach, &key, handler.clone());
        ok(handle(&for_attach, &key, id))
    });

    let for_emit = Rc::clone(registry);
    put(on, "emit", move |_, args| {
        let name = event_name(args.first(), "oslo.on.emit")?;
        let payload = args.get(1).cloned().unwrap_or(Value::Nil);
        let mut ran = 0;
        for handler in handlers(&for_emit, &format!("user:{name}")) {
            // The name as the second argument, so one handler can serve several events — the same
            // thing neovim's `match` gives a `User` autocommand.
            match crate::lua::engine::call_here(&handler, vec![payload.clone(), Value::str(&name)])
            {
                Ok(_) => ran += 1,
                // Reported and stepped over, like every other hook: one plugin's broken handler
                // must not stop another's from hearing the event.
                Err(problem) => eprintln!("oslo: {name}: {problem}"),
            }
        }
        // How many heard it, which is the only way an emitter can tell "nobody is listening" from
        // "everybody failed".
        ok(Value::int(ran))
    });
}

/// An event name, or why it is not one.
///
/// **Refused rather than accepted loosely**, because the failure it prevents is silent: a typo in
/// `oslo.on.user("notes:svaed", …)` subscribes to an event nothing will ever emit, and the plugin
/// simply never reacts. A name with a space or a newline in it is a mistake that reads as one.
fn event_name(value: Option<&Value>, called: &str) -> Result<String, LuaError> {
    let Some(Value::Str(name)) = value else {
        return Err(LuaError::new(format!(
            "{called}: the first argument is the event's name"
        )));
    };
    let name = name.to_string();
    let shaped = !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || ":._-/".contains(c));
    if !shaped {
        return Err(LuaError::new(format!(
            "{called}: {name:?} is not an event name: letters, digits, and `: . _ - /`"
        )));
    }
    Ok(name)
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
///
/// **Where a plugin waiting on a hook wakes up.** Every path that fires a hook comes through here to
/// find its handlers, so this is the one place that can load a plugin *before* the firing it asked
/// for — a plugin woken afterwards would hear the next one instead, which reads as "it works,
/// sometimes". `load_for_hook` is a length check when nothing is waiting, which is every session
/// that has no such plugin.
pub(crate) fn handlers(registry: &Registry, name: &str) -> Vec<Value> {
    #[cfg(feature = "plugin")]
    crate::plugin::load_for_hook(name);
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

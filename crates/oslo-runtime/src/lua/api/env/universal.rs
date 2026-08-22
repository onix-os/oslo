//! `oslo.env.universal` — a variable that outlives this shell and reaches the others.
//!
//! ```lua
//! oslo.env.universal_set("THEME", "dark")          -- here, everywhere, and next time
//! oslo.env.universal_set("EDITOR", "hx", { export = true })
//! oslo.env.universal("THEME")   --> { value = "dark", exported = false }
//! oslo.env.universal()          --> { THEME = { … }, EDITOR = { … } }
//! oslo.env.universal_erase("THEME")
//! ```
//!
//! # Reading is pure and writing is not, which is the whole shape of this file
//!
//! The store is a flock'd file with no `Environment` anywhere in its path, so **reading works from
//! a registered builtin, an answering hook, and inside the line editor** — the places the rest of
//! `oslo.env` refuses.
//!
//! Writing cannot be, and the reason is worth stating because it is the thing an obvious
//! implementation gets wrong. `crate::env::builtins::universal` does **four** things per set, not
//! one: it writes the file, applies the value to *this* shell, records that the value came from the
//! file, and announces the change. A binding that called `universal::set` alone would write to disk
//! and leave the shell it was called from stale until the next prompt — a command that appears to
//! do nothing until you open a new terminal. So the writes take the lock and do all four.
//!
//! A builtin that wants to persist something returns `{ universal = { NAME = value } }` instead;
//! the effects table is applied by a caller that holds the environment. See
//! [`crate::lua::api::builtin::effects`].

use super::super::util::{failed, ok, put, record, text};
use crate::lua::engine::borrow_env;
use oslo_base::value::{LuaError, Table, Value};
use oslo_shell::env::Environment;
use oslo_shell::env::announce::{Change, Scope, Source, announce};
use oslo_shell::env::scope::is_valid_identifier;
use oslo_shell::env::universal;
use std::sync::{Arc, Mutex};

/// Add the universal entries to `oslo.env`.
pub fn install(oslo: &mut Table, env: &Arc<Mutex<Environment>>) {
    reading(oslo);
    writing(oslo, env);
}

fn reading(oslo: &mut Table) {
    // oslo.env.universal([name]) -> one record, or every one keyed by name
    //
    // No lock: this reads the file, so it answers from anywhere. A name that is not there is `nil`
    // rather than an empty record, so `if oslo.env.universal("THEME") then` reads correctly.
    put(oslo, "universal", |_, args| {
        let stored = universal::load();
        let Some(name) = args.first().filter(|v| !matches!(v, Value::Nil)) else {
            let mut all = Table::new();
            for (name, var) in stored {
                all.set(Value::str(name), as_record(&var));
            }
            return ok(Value::table(all));
        };
        let name = match name {
            Value::Str(s) => s.to_string(),
            other => {
                return Err(LuaError::new(format!(
                    "oslo.env.universal: argument #1 must be a string, got {}",
                    other.type_name()
                )));
            }
        };
        ok(stored.get(&name).map_or(Value::Nil, as_record))
    });
}

fn writing(oslo: &mut Table, env: &Arc<Mutex<Environment>>) {
    let env_set = Arc::clone(env);
    // oslo.env.universal_set(name, value, { export = false }) -> true, or nil + why
    put(oslo, "universal_set", move |_, args| {
        let name = text(&args, 1, "oslo.env.universal_set")?;
        let value = text(&args, 2, "oslo.env.universal_set")?;
        if !is_valid_identifier(&name) {
            return failed(
                "oslo.env.universal_set",
                format!("{name}: not a valid name"),
            );
        }
        let exported = exported_flag(args.get(2));
        if let Err(why) = universal::set(&name, &value, exported) {
            return failed("oslo.env.universal_set", why);
        }
        // The other three, in the order the builtin does them.
        borrow_env(&env_set)?.set_var(&name, &value, exported);
        universal::note_applied(&name, &value);
        announce(
            &name,
            Change::Set { exported },
            Scope::Universal,
            Source::Local,
        );
        ok(Value::Bool(true))
    });

    let env_erase = Arc::clone(env);
    // oslo.env.universal_erase(name) -> true, false when it was not one, or nil + why
    put(oslo, "universal_erase", move |_, args| {
        let name = text(&args, 1, "oslo.env.universal_erase")?;
        match universal::erase(&name) {
            Ok(false) => ok(Value::Bool(false)),
            Err(why) => failed("oslo.env.universal_erase", why),
            Ok(true) => {
                // Gone from this shell too, or the value lingers here and nowhere else.
                borrow_env(&env_erase)?.unset_var(&name);
                universal::forget_applied(&name);
                announce(&name, Change::Erased, Scope::Universal, Source::Local);
                ok(Value::Bool(true))
            }
        }
    });
}

/// One stored variable as `{ value = …, exported = … }`.
fn as_record(var: &universal::Universal) -> Value {
    record(vec![
        ("value", Value::str(&var.value)),
        ("exported", Value::Bool(var.exported)),
    ])
}

/// `{ export = true }` in argument three, defaulting to false.
///
/// **The opposite default from `oslo.env.set`, and deliberately.** A universal variable is the one
/// most likely to be a preference rather than something a child needs, and it is written once and
/// read for months — so the setting that leaks it into every process on the machine is the one that
/// should have to be asked for.
fn exported_flag(value: Option<&Value>) -> bool {
    match value {
        Some(Value::Table(opts)) => matches!(opts.borrow().get_str("export"), Value::Bool(true)),
        _ => false,
    }
}

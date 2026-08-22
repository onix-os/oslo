//! `oslo.env` — shell and environment variables.
//!
//! A file of its own, like every other namespace, and it was the last one without one: these lived
//! inline in [`super`] while `oslo.fs`, `oslo.path`, `oslo.re` and the rest each had a module. The
//! split is not tidying — `mod.rs` had reached 596 of the 600 lines `scripts/check-loc.sh` allows,
//! so every namespace that wanted a line there was blocked behind this one.
//!
//! # These take the lock, and that is the whole distinction
//!
//! Everything here goes through [`borrow_env`], so everything here raises *shell state is busy*
//! inside a registered builtin or an answering hook. That is not true of most of `oslo.*` — the
//! lock is 28 functions across six files, not the whole API — and the difference is worth knowing
//! before reaching for a workaround. See `docs/features/your-own-tools.md`.

use super::envlist;
use super::util::{put, text};
use crate::lua::engine::borrow_env;
use oslo_base::value::{Table, Value};
use oslo_shell::env::Environment;
use std::sync::{Arc, Mutex};

/// Install `oslo.env`.
pub fn install(oslo: &mut Table, env: &Arc<Mutex<Environment>>) {
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

    // `$PATH` and its relatives as the lists they are. See [`super::envlist`].
    envlist::install(oslo, env);
}

//! Running a Lua builtin, with the shell it is standing in.
//!
//! # The line this file exists to change
//!
//! The registration used to read:
//!
//! ```ignore
//! guard.register_dynamic_builtin(&declared.name, move |_env, args| {
//!     Ok(call_lua_builtin(&key, args))
//! });
//! ```
//!
//! `_env` is a live, exclusive `&mut Environment`, bound and discarded — and the registry clones the
//! callback out precisely so the closure *can* hold one, which its own comment says. The capability
//! was designed in and then dropped, and every "shell state is busy" complaint about a builtin
//! traces back to the Lua body reaching for what that underscore threw away.
//!
//! So the record is built from that borrow before the call, and the effects are applied to it
//! after. No lock is taken, because none needs to be.

use super::effects::{self, Allow};
use oslo_base::value::{Table, Value};
use oslo_shell::env::Environment;
use oslo_shell::env::view::{self, Wants};

/// Run the callback registered for `name`, handed its argv and the shell.
pub(crate) fn call(env: &mut Environment, name: &str, wants: Wants, args: &[String]) -> i32 {
    let Some((interp, registry)) = crate::lua::engine::active() else {
        eprintln!("oslo: {name}: no Lua interpreter on this thread");
        return 127;
    };
    let key = format!("{}{}", crate::lua::engine::BUILTIN_KEY_PREFIX, name);
    let Some(callback) = registry.borrow().get(&key).cloned() else {
        eprintln!("oslo: {name}: no Lua callback registered");
        return 127;
    };

    // Argv reaches the callback as one table, `argv[1]` its own name — unchanged, and the one
    // contract deliberately *not* made symmetrical with the record's 1-based lists.
    let mut argv = Table::new();
    for (i, arg) in args.iter().enumerate() {
        argv.set(Value::int(i as i64 + 1), Value::str(arg));
    }
    let shell = view::record(env, name, wants);

    match interp.call_function(&callback, vec![Value::table(argv), shell]) {
        Ok(values) => match values.first() {
            Some(Value::Table(returned)) => {
                match effects::parse(&returned.borrow(), Allow::All, name) {
                    Ok(effects) => effects.apply(env),
                    Err(e) => {
                        eprintln!("oslo: {name}: {e}");
                        1
                    }
                }
            }
            // Every scalar spelling still works: nothing, true, false, a number, a string.
            other => super::status::from_lua(other),
        },
        Err(e) => {
            eprintln!("oslo: {name}: {e}");
            1
        }
    }
}

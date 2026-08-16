//! What `oslo.register_builtin` was told.
//!
//! ```lua
//! oslo.register_builtin("note", f)                     -- the old form, unchanged
//! oslo.register_builtin{ name = "note", run = f,       -- the same, plus what it is
//!   desc     = "write a note down",
//!   complete = function(prior, word) … end }
//! ```
//!
//! # Why a command should be able to describe itself
//!
//! The two-argument form takes a name and a function, and nothing else — so a builtin a config adds
//! cannot say what it is for, and a plugin wanting completion had to reach a *second* table
//! (`oslo.completion.for_command`) that has nothing to do with where the command was declared.
//! Neovim's `nvim_create_user_command` carries `desc`, `nargs` and `complete` on the command itself
//! for the same reason: one declaration is one thing to keep right.
//!
//! `nargs` is deliberately absent. In a shell, arguments are words and every builtin already parses
//! its own; a declared arity would be a second, weaker parser that disagrees with the first.

use super::util::put;
use oslo_base::value::{Table, Value};
use oslo_lua::{Interp, LuaError, LuaResult};
use std::cell::RefCell;
use std::collections::HashMap;

/// One builtin, as its declaration described it.
#[derive(Debug)]
pub(super) struct Declared {
    pub name: String,
    pub run: Value,
    pub desc: Option<String>,
    pub complete: Option<Value>,
}

thread_local! {
    /// What each registered builtin said about itself, for `oslo.builtins()`.
    static DECLARED: RefCell<HashMap<String, Option<String>>> = RefCell::new(HashMap::new());
}

/// Read either form of the call.
pub(super) fn declaration(args: &[Value]) -> LuaResult<Declared> {
    // The table form is a single table argument; the old form is a string and a function. Told
    // apart by the first argument's type, so neither can be mistaken for the other.
    if let Some(Value::Table(spec)) = args.first() {
        let spec = spec.borrow();
        let Value::Str(name) = spec.get(&Value::str("name")) else {
            return Err(LuaError::new(
                "oslo.register_builtin: `name` must be a string",
            ));
        };
        let run = spec.get(&Value::str("run"));
        if !matches!(run, Value::Function(_)) {
            return Err(LuaError::new(
                "oslo.register_builtin: `run` must be a function",
            ));
        }
        let complete = match spec.get(&Value::str("complete")) {
            Value::Nil => None,
            it @ Value::Function(_) => Some(it),
            _ => {
                return Err(LuaError::new(
                    "oslo.register_builtin: `complete` must be a function",
                ));
            }
        };
        return Ok(Declared {
            name: named(&name)?,
            run,
            desc: match spec.get(&Value::str("desc")) {
                Value::Str(text) => Some(text.to_string()),
                _ => None,
            },
            complete,
        });
    }

    let Some(Value::Str(name)) = args.first() else {
        return Err(LuaError::new(
            "oslo.register_builtin: a name and a function, or one table",
        ));
    };
    let Some(run @ Value::Function(_)) = args.get(1) else {
        return Err(LuaError::new(
            "oslo.register_builtin: the second argument must be a function",
        ));
    };
    Ok(Declared {
        name: named(name)?,
        run: run.clone(),
        desc: None,
        complete: None,
    })
}

fn named(name: &str) -> LuaResult<String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err(LuaError::new(
            "oslo.register_builtin: the builtin name must not be empty",
        ));
    }
    Ok(name)
}

/// Keep what the declaration said, and wire its completion where completion is read.
///
/// **`complete` is put in `oslo.completion.for_command` rather than in a table of its own.** That is
/// where `lua::columns` already looks, so a declared completion is the same thing a config could
/// have written by hand — one mechanism, reachable two ways, instead of two mechanisms that have to
/// agree.
pub(super) fn remember(interp: &Interp, declared: &Declared) {
    DECLARED.with(|slot| {
        slot.borrow_mut()
            .insert(declared.name.clone(), declared.desc.clone())
    });
    let Some(complete) = &declared.complete else {
        return;
    };
    let Value::Table(oslo) = interp.global("oslo") else {
        return;
    };
    let completion = oslo.borrow().get(&Value::str("completion"));
    let Value::Table(completion) = completion else {
        return;
    };
    let for_command = completion.borrow().get(&Value::str("for_command"));
    if let Value::Table(for_command) = for_command {
        for_command
            .borrow_mut()
            .set(Value::str(&declared.name), complete.clone());
    }
}

/// Add `oslo.builtins()` — what a config has declared, and what each said it is for.
///
/// The counterpart of `oslo.tools()`, and it exists for the same reason: to tell a builtin that
/// failed to register from one whose name was misspelled.
pub(super) fn install(oslo: &mut Table) {
    put(oslo, "builtins", |_, _| {
        let mut listed: Vec<(String, Option<String>)> = DECLARED.with(|slot| {
            slot.borrow()
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect()
        });
        listed.sort();
        let mut out = Table::new();
        for (at, (name, desc)) in listed.into_iter().enumerate() {
            let mut row = Table::new();
            row.set(Value::str("name"), Value::str(&name));
            row.set(
                Value::str("desc"),
                desc.map(|d| Value::str(&d)).unwrap_or(Value::Nil),
            );
            out.set(Value::int(at as i64 + 1), Value::table(row));
        }
        Ok(vec![Value::table(out)])
    });
}

#[cfg(test)]
#[path = "builtin/tests.rs"]
mod tests;

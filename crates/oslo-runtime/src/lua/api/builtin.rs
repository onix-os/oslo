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

pub(super) mod call;
mod effects;
pub(crate) mod status;

use super::util::put;
use oslo_base::value::{LuaError, LuaResult};
use oslo_base::value::{Table, Value};
use oslo_luavm::Host;
use oslo_shell::env::view::Wants;
use std::cell::RefCell;
use std::collections::HashMap;

/// One builtin, as its declaration described it.
#[derive(Debug)]
pub(super) struct Declared {
    pub name: String,
    pub run: Value,
    pub desc: Option<String>,
    pub complete: Option<Value>,
    /// Which of the expensive collections the shell record should gather. Read once, here.
    pub wants: Wants,
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
        let Value::Str(name) = spec.get_str("name") else {
            return Err(LuaError::new(
                "oslo.register_builtin: `name` must be a string",
            ));
        };
        let run = spec.get_str("run");
        if !matches!(run, Value::Function(_)) {
            return Err(LuaError::new(
                "oslo.register_builtin: `run` must be a function",
            ));
        }
        let complete = match spec.get_str("complete") {
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
            desc: match spec.get_str("desc") {
                Value::Str(text) => Some(text.to_string()),
                _ => None,
            },
            complete,
            wants: wanted(&spec.get_str("wants"))?,
        });
    }

    Err(LuaError::new(
        "oslo.register_builtin: one table — { name = \"…\", run = function(argv, shell) … end }",
    ))
}

/// Read `wants = { "vars", … }`, once, at declaration.
///
/// **An unknown name is refused here rather than at call time.** A typo would otherwise be a field
/// that is mysteriously absent from the record much later, in somebody else's builtin.
fn wanted(value: &Value) -> LuaResult<Wants> {
    match value {
        Value::Nil => Ok(Wants::default()),
        Value::Table(list) => {
            let mut wants = Wants::default();
            for (_, entry) in list.borrow().pairs() {
                let Value::Str(name) = &entry else {
                    return Err(LuaError::new(
                        "oslo.register_builtin: wants: every entry must be a string".to_string(),
                    ));
                };
                match Wants::bit(name) {
                    Some(bit) => wants = wants.with(bit),
                    None => {
                        return Err(LuaError::new(format!(
                            "oslo.register_builtin: wants: `{name}` is not one of {}",
                            Wants::NAMES.join(", ")
                        )));
                    }
                }
            }
            Ok(wants)
        }
        other => Err(LuaError::new(format!(
            "oslo.register_builtin: wants must be a list of names, got {}",
            other.type_name()
        ))),
    }
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
pub(super) fn remember(host: &dyn Host, declared: &Declared) {
    DECLARED.with(|slot| {
        slot.borrow_mut()
            .insert(declared.name.clone(), declared.desc.clone())
    });
    let Some(complete) = &declared.complete else {
        return;
    };
    // Through the host rather than by fetching the table: what `global` answers is a copy of the
    // VM's table, so inserting into it would land nowhere.
    host.set_field(
        &["oslo", "completion", "for_command", &declared.name],
        complete.clone(),
    );
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
            row.set_str("name", Value::str(&name));
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

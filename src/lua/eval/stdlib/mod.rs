//! The standard library oslo's Lua provides, and the loud refusals for what it does not.
//!
//! Two rules govern this module.
//!
//! **Everything unimplemented is present and erroring.** `coroutine` is a real table whose every
//! function raises `coroutine.create is not implemented in oslo's Lua`. Leaving it `nil` would
//! turn the first use into `attempt to index a nil value`, which tells the reader nothing about
//! what happened or why. See [`stub`].
//!
//! **Nothing here reimplements the core.** Filesystem, process and job functions belong to
//! `crate::env` and `crate::exec`, and are exposed under `oslo.*` — one implementation, reached
//! from both languages.

mod base;
mod math;
pub mod pattern;
mod string;
mod stub;
mod table;

use super::value::{Function, Table, Value};
use super::{Interp, LuaResult};
use std::rc::Rc;

/// Wrap a Rust function as a Lua value.
pub fn native(
    name: &'static str,
    call: impl Fn(&Interp, Vec<Value>) -> LuaResult<Vec<Value>> + 'static,
) -> Value {
    Value::Function(Rc::new(Function::Native {
        name,
        call: Box::new(call),
    }))
}

/// Build a namespace table from `(name, function)` pairs.
pub fn module(entries: Vec<(&'static str, Value)>) -> Value {
    let mut table = Table::new();
    for (name, value) in entries {
        table.set(Value::str(name), value);
    }
    Value::table(table)
}

/// Populate the globals of a fresh interpreter.
pub fn install(interp: &Interp) {
    base::install(interp);
    string::install(interp);
    table::install(interp);
    math::install(interp);
    stub::install(interp);

    // `_G` is the global table itself, which scripts use for reflection and for deliberate
    // dynamic access. It is a real self-reference, and the cycle it creates is exactly the kind
    // this evaluator cannot collect — one table for the life of the process, which is fine.
    let globals = Value::Table(Rc::clone(&interp.globals));
    interp.set_global("_G", globals);
    interp.set_global("_VERSION", Value::str("Lua 5.4"));
}

/// Argument `n` (1-based), or nil.
pub fn arg(args: &[Value], n: usize) -> Value {
    args.get(n - 1).cloned().unwrap_or(Value::Nil)
}

/// Argument `n` as a string, with Lua's error wording when it is not one.
pub fn arg_str(args: &[Value], n: usize, function: &str) -> LuaResult<String> {
    match arg(args, n) {
        Value::Str(s) => Ok(s.to_string()),
        // Numbers coerce to strings in every string function, which is why `("x"):rep(3)` and
        // `string.rep(3, 2)` both work.
        Value::Number(num) => Ok(num.to_string()),
        other => Err(super::LuaError::new(format!(
            "bad argument #{n} to '{function}' (string expected, got {})",
            other.type_name()
        ))),
    }
}

/// Argument `n` as an integer, with Lua's error wording.
pub fn arg_int(args: &[Value], n: usize, function: &str) -> LuaResult<i64> {
    arg(args, n)
        .as_number()
        .and_then(|v| v.as_int())
        .ok_or_else(|| {
            super::LuaError::new(format!(
                "bad argument #{n} to '{function}' (number expected, got {})",
                arg(args, n).type_name()
            ))
        })
}

/// Argument `n` as a table.
pub fn arg_table(
    args: &[Value],
    n: usize,
    function: &str,
) -> LuaResult<Rc<std::cell::RefCell<Table>>> {
    match arg(args, n) {
        Value::Table(t) => Ok(t),
        other => Err(super::LuaError::new(format!(
            "bad argument #{n} to '{function}' (table expected, got {})",
            other.type_name()
        ))),
    }
}

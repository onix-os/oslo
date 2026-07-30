//! Shared plumbing for the `oslo.*` namespaces: registering functions, reading arguments, and the
//! one failure shape they all use.

use crate::lua::eval::value::{Function, Table, Value};
use crate::lua::eval::{Interp, LuaError, LuaResult};
use std::rc::Rc;

/// Wrap a Rust function as a Lua value.
pub fn native(
    name: &'static str,
    f: impl Fn(&Interp, Vec<Value>) -> LuaResult<Vec<Value>> + 'static,
) -> Value {
    Value::Function(Rc::new(Function::Native {
        name,
        call: Box::new(f),
    }))
}

/// Register a function in a table under `name`.
pub fn put(
    table: &mut Table,
    name: &'static str,
    f: impl Fn(&Interp, Vec<Value>) -> LuaResult<Vec<Value>> + 'static,
) {
    table.set(Value::str(name), native(name, f));
}

/// Argument `n` (1-based) as a string, refusing anything that is not one.
///
/// Numbers count: `oslo.path.join("a", 1)` is a reasonable thing to write, and Lua coerces the
/// same way in `..`. A table or a function does not, because reaching a filesystem call as
/// `table: 0x55f…` is never what was meant.
pub fn text(args: &[Value], n: usize, function: &str) -> LuaResult<String> {
    match args.get(n - 1) {
        Some(Value::Str(s)) => Ok(s.to_string()),
        Some(Value::Number(x)) => Ok(x.to_string()),
        other => Err(LuaError::new(format!(
            "{function}: argument #{n} must be a string, got {}",
            other.map_or("no value", Value::type_name)
        ))),
    }
}

/// Argument `n` as a string, or `None` when it was not given.
pub fn opt_text(args: &[Value], n: usize, function: &str) -> LuaResult<Option<String>> {
    match args.get(n - 1) {
        None | Some(Value::Nil) => Ok(None),
        _ => text(args, n, function).map(Some),
    }
}

/// Argument `n` as an integer.
pub fn int(args: &[Value], n: usize, function: &str) -> LuaResult<i64> {
    args.get(n - 1)
        .and_then(Value::as_number)
        .and_then(|v| v.as_int())
        .ok_or_else(|| {
            LuaError::new(format!(
                "{function}: argument #{n} must be a number, got {}",
                args.get(n - 1).map_or("no value", Value::type_name)
            ))
        })
}

/// The success half of the `nil, message` convention.
pub fn ok(value: Value) -> LuaResult<Vec<Value>> {
    Ok(vec![value])
}

/// The failure half: `nil` plus a message, the shape `io.open` uses.
///
/// Fallible calls answer rather than raise, because a missing file is a condition a script
/// handles, not a bug in it. A raised error is kept for a *caller* mistake — a missing argument,
/// a name that cannot exist — which is a bug in the script and should stop it.
pub fn failed(context: &str, e: impl std::fmt::Display) -> LuaResult<Vec<Value>> {
    Ok(vec![Value::Nil, Value::str(format!("{context}: {e}"))])
}

/// Build a table from `(key, value)` pairs.
pub fn record(entries: Vec<(&str, Value)>) -> Value {
    let mut table = Table::new();
    for (key, value) in entries {
        table.set(Value::str(key), value);
    }
    Value::table(table)
}

/// Build a sequence table.
pub fn list(values: impl IntoIterator<Item = Value>) -> Value {
    let mut table = Table::new();
    for (i, value) in values.into_iter().enumerate() {
        table.set(Value::int(i as i64 + 1), value);
    }
    Value::table(table)
}

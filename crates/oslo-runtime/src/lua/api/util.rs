//! Shared plumbing for the `oslo.*` namespaces: registering functions, reading arguments, and the
//! one failure shape they all use.

use oslo_base::value::{Function, LuaError, LuaResult, Table, Value};
use oslo_luavm::{Host, Native};
use std::rc::Rc;

/// Wrap a Rust function as a Lua value.
///
/// The result is an ordinary shell value carrying an opaque payload, which is what lets a namespace
/// be *built* — here and in forty other files — with no VM in scope. It becomes a callback the
/// instant it crosses into the engine, and not before.
pub fn native(
    name: &'static str,
    f: impl Fn(&dyn Host, Vec<Value>) -> LuaResult<Vec<Value>> + 'static,
) -> Value {
    Value::Function(Rc::new(Function::Held(Rc::new(Native {
        name,
        call: Box::new(f),
    }))))
}

/// Register a function in a table under `name`.
pub fn put(
    table: &mut Table,
    name: &'static str,
    f: impl Fn(&dyn Host, Vec<Value>) -> LuaResult<Vec<Value>> + 'static,
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

/// Argument `n` as bytes, for a call that writes what it is given rather than reading it.
///
/// **Separate from [`text`], because the two want opposite things of a byte string.** A path, a
/// variable name or a command word has to be text, and `text` refusing a run of arbitrary bytes is
/// what catches the caller who passed the contents of a file where a name goes. What
/// `oslo.fs.write` is handed, on the other hand, is *content*: refusing it because it is not UTF-8
/// would mean the only thing `oslo.fs.read` cannot round-trip is exactly the thing it was taught to
/// read. Numbers count here too, for the same reason they do in `text`.
pub fn raw(args: &[Value], n: usize, function: &str) -> LuaResult<Vec<u8>> {
    match args.get(n - 1) {
        Some(value) if value.as_bytes().is_some() => Ok(value.as_bytes().unwrap_or(&[]).to_vec()),
        Some(Value::Number(x)) => Ok(x.to_string().into_bytes()),
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
    Ok(vec![
        Value::Nil,
        super::problem::new(format!("{context}: {e}"), Vec::new()),
    ])
}

/// The same, where the call was about a path and the failure came from the operating system.
///
/// The answer carries `path`, `kind` and `code` alongside the message, so a caller can ask what
/// went wrong instead of matching the sentence that says so. See [`super::problem`].
pub fn failed_path(path: &str, e: &std::io::Error) -> LuaResult<Vec<Value>> {
    let mut facts = vec![
        ("path", Value::str(path)),
        ("kind", Value::str(super::problem::kind_of(e.kind()))),
    ];
    if let Some(code) = e.raw_os_error() {
        facts.push(("code", Value::int(code as i64)));
    }
    Ok(vec![
        Value::Nil,
        super::problem::new(format!("{path}: {e}"), facts),
    ])
}

/// A filesystem call with two ends — `rename`, `copy` — where `path` alone would not say which one
/// failed, so the answer carries both.
pub fn failed_between(from: &str, to: &str, e: &std::io::Error) -> LuaResult<Vec<Value>> {
    let mut facts = vec![
        ("path", Value::str(from)),
        ("to", Value::str(to)),
        ("kind", Value::str(super::problem::kind_of(e.kind()))),
    ];
    if let Some(code) = e.raw_os_error() {
        facts.push(("code", Value::int(code as i64)));
    }
    Ok(vec![
        Value::Nil,
        super::problem::new(format!("{from} -> {to}: {e}"), facts),
    ])
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

#[cfg(test)]
#[path = "util/probe.rs"]
pub(super) mod probe;

/// The longest any single wait may be: a day.
///
/// Not a limit anybody should meet. It exists because a Lua number has no ceiling and
/// `Duration::from_secs_f64` **panics** on one a `Duration` cannot hold — so `oslo.after(1e30, f)`
/// aborted the shell, uncatchably, from a config.
pub const LONGEST_WAIT: std::time::Duration = std::time::Duration::from_secs(24 * 60 * 60);

/// Milliseconds from Lua as a `Duration`, clamped so the conversion cannot panic.
///
/// **Clamped before converting.** Every site that got this wrong had a ceiling written down and
/// applied it with `.min(…)` *after* `from_secs_f64`, which is the call that dies — so the guard
/// was there and did nothing. A negative or non-finite number is no wait at all.
pub fn wait_from_millis(millis: f64) -> std::time::Duration {
    seconds_to_duration(millis / 1000.0)
}

/// The same, for the entry points that are given seconds.
pub fn seconds_to_duration(seconds: f64) -> std::time::Duration {
    if !seconds.is_finite() || seconds <= 0.0 {
        return std::time::Duration::ZERO;
    }
    std::time::Duration::from_secs_f64(seconds.min(LONGEST_WAIT.as_secs_f64()))
}

/// A count or a width from Lua, with a ceiling a terminal could never need.
///
/// A Lua number reaching an allocation is a way to kill the shell from a config:
/// `oslo.ui.rule("-", 1e15)` asked for a petabyte and died with `memory allocation of
/// 1000000000000000 bytes failed`, SIGABRT, on the prompt surface — so a width computed from a bad
/// terminal query took the session with it.
pub const WIDEST: usize = 10_000;

/// Clamp a width or a repeat count to something a terminal could actually mean.
pub fn width_of(n: f64) -> usize {
    if !n.is_finite() || n <= 0.0 {
        return 0;
    }
    (n as usize).min(WIDEST)
}

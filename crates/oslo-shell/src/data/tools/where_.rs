//! `where` — keep the rows a Lua expression is true for.
//!
//! ```text
//! df | where 'free < 1e9'
//! ps | where 'cpu > 10'
//! ls | where 'name:match("%.rs$")'
//! ```
//!
//! **The filter is Lua, not a dialect of its own.** Every structured shell eventually invents a
//! small expression language for filtering, and every one of them then needs an escape hatch when
//! the language runs out — at which point there are two languages to know instead of one. oslo
//! already has Lua at the prompt, so the filter and the escape hatch are the same thing.
//!
//! The row's columns are bound as globals for the duration of the expression, so `free < 1e9`
//! reads the way it looks. `row` is bound too, for a column whose name is not a Lua identifier.

use crate::data::{Record, Val};
use oslo_base::value::{Number, Table, Value};
use oslo_luavm::{Engine, Host};

/// Evaluate `expression` against each row, keeping the ones it is true for.
///
/// The expression is parsed **once**, not once per row: a `ps` table is a few hundred rows and
/// re-parsing the same six characters for each of them is the difference between a filter and a
/// pause.
///
/// A row whose expression fails — a typo, a comparison against a column that is not there — is
/// *dropped*, and the failure is reported once rather than per row. Keeping such rows would be
/// worse: a filter that quietly passes everything through when it breaks is how a pipeline ending
/// in `rm` removes the wrong thing.
pub fn filter(rows: &[Record], expression: &str) -> (Vec<Record>, Option<String>) {
    // The prompt's engine when there is one, so a filter sees the same globals and functions a
    // config defined. In a script or a `-c` command there is none, and a filter must still work —
    // so one is made for the occasion. Made once for the whole filter, not once per row.
    let engine = session_engine();
    let source = format!("return ({expression})");
    let compiled = match engine.load(&source, "where") {
        Ok(compiled) => compiled,
        Err(e) => return (Vec::new(), Some(format!("where: {e}"))),
    };

    let mut kept = Vec::new();
    let mut failure = None;
    for row in rows {
        // The columns are visible as themselves for the length of one evaluation, so
        // `free < 1e9` reads the way it looks.
        let names: Vec<String> = row.columns().to_vec();
        for (name, value) in names.iter().zip(row.values()) {
            engine.set_global(name, to_lua(value));
        }
        engine.set_global("row", to_lua(&Val::Record(row.clone())));

        match engine.call_function(&compiled, Vec::new()) {
            Ok(values) => {
                if values.first().is_some_and(|v| v.truthy()) {
                    kept.push(row.clone());
                }
            }
            Err(e) => {
                if failure.is_none() {
                    failure = Some(format!("where: {e}"));
                }
            }
        }

        // Cleared again, or a column called `x` would still be a global at the next prompt.
        for name in &names {
            engine.set_global(name, Value::Nil);
        }
        engine.set_global("row", Value::Nil);
    }
    (kept, failure)
}

/// The session's engine, or a fresh one when the filter is running outside a session.
fn session_engine() -> std::rc::Rc<Engine> {
    oslo_luavm::current::handle().unwrap_or_else(|| std::rc::Rc::new(Engine::new()))
}

/// A pipeline value as Lua sees it.
fn to_lua(value: &Val) -> Value {
    match value {
        Val::Null => Value::Nil,
        Val::Bool(b) => Value::Bool(*b),
        Val::Int(i) => Value::int(*i),
        // A size is a number in Lua, so `free < 1e9` is arithmetic rather than string comparison —
        // which is the whole reason a size is a distinct kind rather than the text `4.2G`.
        Val::Size(bytes) => Value::int(*bytes as i64),
        Val::Duration(ns) | Val::Time(ns) => Value::int(*ns),
        Val::Float(f) => Value::Number(Number::Float(*f)),
        Val::Str(s) => Value::str(s),
        Val::Bytes(b) => Value::int(b.len() as i64),
        Val::Error(_) => Value::Nil,
        Val::List(items) => {
            let mut t = Table::new();
            for (i, item) in items.iter().enumerate() {
                t.set(Value::int(i as i64 + 1), to_lua(item));
            }
            Value::Table(std::rc::Rc::new(std::cell::RefCell::new(t)))
        }
        Val::Record(record) => {
            let mut t = Table::new();
            for (name, value) in record.columns().iter().zip(record.values()) {
                t.set(Value::str(name), to_lua(value));
            }
            Value::Table(std::rc::Rc::new(std::cell::RefCell::new(t)))
        }
    }
}

/// Run an expression once per row, for what it does rather than what it answers.
///
/// The pressure valve. Without it, every unmet need becomes a request for operator number forty;
/// with it, the answer is "write the Lua". It costs an adapter rather than an interpreter, because
/// the interpreter is already here.
///
/// Answers the first failure, or `None` if every row ran cleanly.
pub fn for_each(rows: &[Record], expression: &str) -> Option<String> {
    // Wrapped in a `do ... end` so a statement is as welcome as an expression: `each 'print(name)'`
    // is a call, and `each 'x = x + n'` is an assignment, and neither should need different syntax.
    let engine = session_engine();
    let source = format!("do {expression} end");
    let compiled = match engine.load(&source, "each") {
        Ok(compiled) => compiled,
        Err(e) => return Some(format!("each: {e}")),
    };

    let mut failure = None;
    for row in rows {
        let names: Vec<String> = row.columns().to_vec();
        for (name, value) in names.iter().zip(row.values()) {
            engine.set_global(name, to_lua(value));
        }
        engine.set_global("row", to_lua(&Val::Record(row.clone())));
        if let Err(e) = engine.call_function(&compiled, Vec::new())
            && failure.is_none()
        {
            failure = Some(format!("each: {e}"));
        }
        for name in &names {
            engine.set_global(name, Value::Nil);
        }
        engine.set_global("row", Value::Nil);
    }
    failure
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows() -> Vec<Record> {
        vec![
            Record::from_pairs([
                ("mount", Val::Str("/".into())),
                ("free", Val::Size(500_000_000)),
            ]),
            Record::from_pairs([
                ("mount", Val::Str("/home".into())),
                ("free", Val::Size(9_000_000_000)),
            ]),
        ]
    }

    fn as_int(value: &Value) -> Option<i64> {
        match value {
            Value::Number(n) => n.as_int(),
            _ => None,
        }
    }

    fn as_text(value: &Value) -> Option<String> {
        match value {
            Value::Str(s) => Some(s.to_string()),
            _ => None,
        }
    }

    /// A size compares as a number, which is exactly what `ls -lh | sort` cannot do.
    #[test]
    fn a_size_is_arithmetic_in_lua() {
        assert_eq!(as_int(&to_lua(&Val::Size(1024))), Some(1024));
    }

    /// Every column reaches Lua under its own name.
    #[test]
    fn a_record_reaches_lua_as_a_table() {
        let Value::Table(t) = to_lua(&Val::Record(rows()[0].clone())) else {
            panic!("a record is a table");
        };
        let t = t.borrow();
        assert_eq!(as_text(&t.get_str("mount")).as_deref(), Some("/"));
        assert_eq!(as_int(&t.get_str("free")), Some(500_000_000));
    }
}

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
//! **For the duration and no longer** — whatever those names held before is put back, because a
//! column is free to be called `type`. See the `Bound` guard below.

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
    // `1GB` is not Lua and cannot be made into Lua, so a unit literal becomes the number the rows
    // carry before any of this is compiled. See `super::units`.
    let expression = super::units::expand(expression);
    let source = format!("return ({expression})");
    let compiled = match engine.load(&source, "where") {
        Ok(compiled) => compiled,
        Err(e) => return (Vec::new(), Some(format!("where: {e}"))),
    };

    let mut kept = Vec::new();
    let mut failure = None;
    for row in rows {
        // The columns are visible as themselves for the length of one evaluation, so
        // `free < 1e9` reads the way it looks. `Bound` puts back whatever they were.
        let _bound = Bound::new(&engine, row);

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
    }
    (kept, failure)
}

/// The globals a row occupies while its expression runs, put back on the way out.
///
/// **Restored, not cleared**, and the difference was a real bug. Setting them back to `nil`
/// afterwards is right only if nothing was there before — and a column is free to be called `type`,
/// which is a Lua *builtin*. `stale | where 'days > 180'` therefore left `type` nil for the rest of
/// the session, and the next thing to call it died with "could not call a nil value" a long way
/// from here. A user's own global goes the same way.
///
/// Every original is read **before** anything is set, so a column named `row` cannot save the value
/// the column binding just wrote over it. Restoring happens in `Drop`, so an expression that fails
/// mid-evaluation does not leave the globals it borrowed lying around.
struct Bound<'a> {
    engine: &'a Engine,
    saved: Vec<(String, Value)>,
}

impl<'a> Bound<'a> {
    fn new(engine: &'a Engine, row: &Record) -> Bound<'a> {
        let names: Vec<String> = row
            .columns()
            .iter()
            .cloned()
            .chain(std::iter::once("row".to_string()))
            .collect();
        let saved: Vec<(String, Value)> = names
            .iter()
            .map(|name| (name.clone(), engine.global(name)))
            .collect();

        for (name, value) in row.columns().iter().zip(row.values()) {
            engine.set_global(name, to_lua(value));
        }
        engine.set_global("row", to_lua(&Val::Record(row.clone())));
        Bound { engine, saved }
    }
}

impl Drop for Bound<'_> {
    fn drop(&mut self) {
        for (name, value) in self.saved.drain(..) {
            self.engine.set_global(&name, value);
        }
    }
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
    let expression = super::units::expand(expression);
    let source = format!("do {expression} end");
    let compiled = match engine.load(&source, "each") {
        Ok(compiled) => compiled,
        Err(e) => return Some(format!("each: {e}")),
    };

    let mut failure = None;
    for row in rows {
        let _bound = Bound::new(&engine, row);
        if let Err(e) = engine.call_function(&compiled, Vec::new())
            && failure.is_none()
        {
            failure = Some(format!("each: {e}"));
        }
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

    /// **A filter puts back the globals it borrowed**, which is not the same as clearing them.
    ///
    /// A column may be called `type`, and `type` is a Lua builtin. Setting the bindings to `nil`
    /// afterwards destroyed it for the rest of the session, and the failure surfaced far away —
    /// `oslo.nix.inputs()` died with "could not call a nil value" only after some earlier `where`
    /// had run over a row with a `type` column.
    #[test]
    fn a_filter_restores_what_it_shadowed() {
        let engine = session_engine();
        engine.set_global("type", Value::Str("a builtin stands here".into()));
        engine.set_global("mine", Value::Str("a global of my own".into()));

        let rows = vec![Record::from_pairs([
            ("type", Val::Str("github".into())),
            ("mine", Val::Str("clobbered".into())),
            ("days", Val::Int(200)),
        ])];
        let (kept, failure) = filter(&rows, "days > 100");
        assert!(failure.is_none(), "{failure:?}");
        assert_eq!(kept.len(), 1);

        assert_eq!(
            as_text(&engine.global("type")).as_deref(),
            Some("a builtin stands here"),
            "the filter left `type` changed"
        );
        assert_eq!(
            as_text(&engine.global("mine")).as_deref(),
            Some("a global of my own")
        );

        // A name that was *not* there before is still not there afterwards — the original
        // intention, which the fix must not lose.
        assert!(matches!(engine.global("days"), Value::Nil));
    }

    /// `each` binds the same way and must put the same things back.
    #[test]
    fn for_each_restores_what_it_shadowed() {
        let engine = session_engine();
        engine.set_global("type", Value::Str("still here".into()));
        let rows = vec![Record::from_pairs([("type", Val::Str("github".into()))])];
        assert!(for_each(&rows, "local _ = type").is_none());
        assert_eq!(
            as_text(&engine.global("type")).as_deref(),
            Some("still here")
        );
    }
}

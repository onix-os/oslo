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

use crate::data::lua::to_lua;
use crate::data::{Record, Val};
use oslo_base::value::Value;
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

/// Evaluate `expression` against each row and keep **what it answers** as the new row.
///
/// ```text
/// ls | map '{ name = name, kb = size / 1024 }'
/// ls | map 'name:upper()'
/// ```
///
/// **The verb oslo did not have.** Every other verb is selection — `where`, `cols`, `first`,
/// `sort-by` keep rows and throw rows away — and `each` runs its Lua for the side effect and
/// produces nothing, so the pipeline ends there. There was no way to *transform* a row at all, and
/// a shell whose structured half cannot map is one where every reshaping question becomes a request
/// for another verb.
///
/// What an expression answers becomes a row by the same rule `from json` already uses for a
/// document that is not an object:
///
/// | the expression answers | the row |
/// |---|---|
/// | a table of named fields | that record, in its own column order |
/// | anything else — a number, a string, a list | one column named `value` |
/// | `nil` | no row, so `map` filters as well as maps |
///
/// A row whose expression raises is dropped and the failure reported once, which is [`filter`]'s
/// rule and is here for the same reason: a transform that quietly passes rows through when it
/// breaks is how the wrong thing ends up in the file at the end of the pipeline.
pub fn map_rows(rows: &[Record], expression: &str) -> (Vec<Record>, Option<String>) {
    let engine = session_engine();
    let expression = super::units::expand(expression);
    let source = format!("return ({expression})");
    let compiled = match engine.load(&source, "map") {
        Ok(compiled) => compiled,
        Err(e) => return (Vec::new(), Some(format!("map: {e}"))),
    };

    let mut out = Vec::new();
    let mut failure = None;
    for row in rows {
        let _bound = Bound::new(&engine, row);
        match engine.call_function(&compiled, Vec::new()) {
            Ok(values) => match crate::data::lua::from_lua(values.first().unwrap_or(&Value::Nil)) {
                // Nothing to say about this row, so it does not produce one.
                Val::Null => {}
                Val::Record(record) if !record.is_empty() => out.push(record),
                other => out.push(Record::from_pairs([("value", other)])),
            },
            Err(e) => {
                if failure.is_none() {
                    failure = Some(format!("map: {e}"));
                }
            }
        }
    }
    (out, failure)
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

    fn as_text(value: &Value) -> Option<String> {
        match value {
            Value::Str(s) => Some(s.to_string()),
            _ => None,
        }
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

    /// A table of named fields becomes the row, in the order the table had them.
    #[test]
    fn map_answers_a_row_per_row() {
        let rows = vec![
            Record::from_pairs([("name", Val::Str("a".into())), ("size", Val::Int(2048))]),
            Record::from_pairs([("name", Val::Str("b".into())), ("size", Val::Int(1024))]),
        ];
        let (mapped, failure) = map_rows(&rows, "{ n = name, kb = size / 1024 }");
        assert!(failure.is_none(), "{failure:?}");
        assert_eq!(mapped.len(), 2);
        assert_eq!(mapped[0].get("n"), Some(&Val::Str("a".into())));
        assert_eq!(mapped[0].get("kb"), Some(&Val::Int(2)));
    }

    /// Anything that is not a record is one column called `value` — the rule `from json` already
    /// uses for a document that is not an object.
    #[test]
    fn a_scalar_becomes_one_column() {
        let rows = vec![Record::from_pairs([("name", Val::Str("a".into()))])];
        let (mapped, failure) = map_rows(&rows, "name:upper()");
        assert!(failure.is_none(), "{failure:?}");
        assert_eq!(mapped[0].get("value"), Some(&Val::Str("A".into())));
        assert_eq!(mapped[0].columns(), ["value"]);
    }

    /// `nil` produces no row, so `map` filters as well as maps.
    #[test]
    fn nil_drops_the_row() {
        let rows = vec![
            Record::from_pairs([("keep", Val::Bool(true))]),
            Record::from_pairs([("keep", Val::Bool(false))]),
        ];
        let (mapped, failure) = map_rows(&rows, "keep and { ok = 1 } or nil");
        assert!(failure.is_none(), "{failure:?}");
        assert_eq!(mapped.len(), 1, "the false row produced nothing");
    }

    /// A row that raises is dropped and the failure reported **once**, not per row — the same rule
    /// `where` follows, because a transform that passes rows through when it breaks is how the
    /// wrong thing reaches the end of a pipeline.
    #[test]
    fn a_raising_row_is_dropped_and_reported_once() {
        let rows = vec![
            Record::from_pairs([("n", Val::Int(1))]),
            Record::from_pairs([("n", Val::Int(2))]),
        ];
        let (mapped, failure) = map_rows(&rows, "nope.field");
        assert!(mapped.is_empty(), "nothing survives a broken transform");
        let message = failure.expect("the failure is reported");
        assert!(message.starts_with("map:"), "got {message}");
    }

    /// `map` borrows the same globals `where` does, and must put them back.
    #[test]
    fn map_restores_what_it_shadowed() {
        let engine = session_engine();
        engine.set_global("type", Value::Str("a builtin stands here".into()));
        let rows = vec![Record::from_pairs([("type", Val::Str("github".into()))])];
        let (mapped, failure) = map_rows(&rows, "{ t = type }");
        assert!(failure.is_none(), "{failure:?}");
        assert_eq!(mapped[0].get("t"), Some(&Val::Str("github".into())));
        assert_eq!(
            as_text(&engine.global("type")).as_deref(),
            Some("a builtin stands here")
        );
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

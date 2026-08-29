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
    let expanded = super::units::expand(expression);
    let source = format!("return ({expanded})");
    let compiled = match engine.load(&source, "where") {
        Ok(compiled) => compiled,
        Err(e) => {
            return (
                Vec::new(),
                Some(expression_error("where", expression, &expanded, e)),
            );
        }
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

/// What went wrong with an expression, said in terms of what was *typed*.
///
/// **The old message blamed a token nobody wrote.** An expression is compiled as `return (…)`, so
/// `where 'size >'` reported
///
/// ```text
/// where: ?: syntax error: parse error at line 1: found "RightParen", expected …
/// ```
///
/// — and that `)` is oslo's own, added by the wrapper. The user is left looking for a bracket they
/// never typed, in a line that has none.
///
/// It also never said *which* expression. A pipeline may hold several, and "syntax error" with no
/// subject is a search rather than a diagnosis.
///
/// So the expression is quoted back, and when [`super::units`] rewrote it — `1GB` is not Lua and
/// becomes its number before compiling — the rewritten form is shown too, because otherwise the
/// parser is complaining about text that appears nowhere on screen.
fn expression_error(
    verb: &str,
    typed: &str,
    expanded: &str,
    problem: impl std::fmt::Display,
) -> String {
    let problem = problem.to_string();
    // **When the offending bracket is provably oslo's own, say what actually happened.**
    //
    // The wrapper contributes exactly one `)`. If the expression as typed contains none, then the
    // `RightParen` the parser tripped over cannot be anything else — the expression ran out before
    // it was finished, and *that* is the thing to say. Where the user did type a bracket the two
    // are no longer distinguishable, so the parser's own words stand.
    if !typed.contains(')') && problem.contains("found \"RightParen\"") {
        return format!("{verb}: {typed}: the expression is not finished");
    }
    let mut out = format!("{verb}: {typed}: {problem}");
    if expanded != typed {
        out.push_str(&format!(" — read as `{expanded}`"));
    }
    out
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
    let expanded = super::units::expand(expression);
    let source = format!("return ({expanded})");
    let compiled = match engine.load(&source, "map") {
        Ok(compiled) => compiled,
        Err(e) => {
            return (
                Vec::new(),
                Some(expression_error("map", expression, &expanded, e)),
            );
        }
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

/// Evaluate `expression` once per row and answer what it produced for each.
///
/// The shared half of `insert`, `update` and `upsert`: they differ only in what they do with the
/// answer, so the Lua — the parse-once, the [`Bound`] guard, the unit rewrite — lives here rather
/// than three times over in `reshape`.
///
/// A row whose expression raises contributes `None` and the failure is reported once.
pub fn compute(rows: &[Record], expression: &str) -> (Vec<Option<Val>>, Option<String>) {
    let engine = session_engine();
    let expression = super::units::expand(expression);
    let source = format!("return ({expression})");
    let compiled = match engine.load(&source, "compute") {
        Ok(compiled) => compiled,
        Err(e) => return (vec![None; rows.len()], Some(e.to_string())),
    };

    let mut out = Vec::with_capacity(rows.len());
    let mut failure = None;
    for row in rows {
        let _bound = Bound::new(&engine, row);
        match engine.call_function(&compiled, Vec::new()) {
            Ok(values) => out.push(Some(crate::data::lua::from_lua(
                values.first().unwrap_or(&Value::Nil),
            ))),
            Err(e) => {
                out.push(None);
                if failure.is_none() {
                    failure = Some(e.to_string());
                }
            }
        }
    }
    (out, failure)
}

/// Evaluate a Lua expression **once** and read its answer as rows.
///
/// This is how a verb gets a *second* input. The pipeline is a line and always has been — there is
/// no `|` shape for "and also read this" — so the other side is named in the one language that is
/// already here:
///
/// ```text
/// ls | lookup 'sh.stat("a", "b")' name
/// ps | append 'oslo.rows.from_json(other)'
/// ```
///
/// Once, not per row: the other side is a table, and re-running the expression for every row of the
/// left one would turn a join into a quadratic pile of subprocesses.
pub fn rows_from(expression: &str) -> Result<Vec<Record>, String> {
    let engine = session_engine();
    let source = format!("return ({expression})");
    let compiled = engine.load(&source, "rows").map_err(|e| format!("{e}"))?;
    let values = engine
        .call_function(&compiled, Vec::new())
        .map_err(|e| format!("{e}"))?;
    let answer = values.first().unwrap_or(&Value::Nil);
    if matches!(answer, Value::Nil) {
        return Err("the expression answered nothing".to_string());
    }
    Ok(crate::data::lua::records_of(answer))
}

/// Fold the whole stream into one value with a Lua expression.
///
/// ```text
/// ls | reduce 'acc + size'            the total, as one row
/// ls | reduce 'acc .. name .. " "'    everything joined
/// ```
///
/// `acc` is the running answer and the row's columns are bound as usual, so the expression reads as
/// the arithmetic it is. It starts at the first row's own value under `--from`, or at zero.
///
/// **Why this and not another summary verb.** `stats` answers the five summaries worth having
/// built in; everything else somebody wants — a weighted mean, a concatenation, a product, a
/// running maximum of one column keyed on another — is a fold, and a fold is one verb. This is the
/// same argument `each` makes: the pressure valve costs an adapter, because the interpreter is
/// already here.
pub fn reduce(
    rows: &[Record],
    expression: &str,
    from: Option<&str>,
) -> (Vec<Record>, Option<String>) {
    let engine = session_engine();
    let expanded = super::units::expand(expression);
    let source = format!("return ({expanded})");
    let compiled = match engine.load(&source, "reduce") {
        Ok(compiled) => compiled,
        Err(e) => {
            return (
                Vec::new(),
                Some(expression_error("reduce", expression, &expanded, e)),
            );
        }
    };

    // The starting value is Lua's, so `--from ''` folds text and the default folds numbers.
    let mut accumulator = match from {
        Some(text) => match text.parse::<i64>() {
            Ok(n) => Value::int(n),
            Err(_) => Value::str(text),
        },
        None => Value::int(0),
    };
    let saved = engine.global("acc");
    let mut failure = None;
    for row in rows {
        let _bound = Bound::new(&engine, row);
        engine.set_global("acc", accumulator.clone());
        match engine.call_function(&compiled, Vec::new()) {
            Ok(values) => accumulator = values.first().cloned().unwrap_or(Value::Nil),
            Err(e) => {
                if failure.is_none() {
                    failure = Some(format!("reduce: {e}"));
                }
            }
        }
    }
    // `acc` is borrowed for the fold exactly as a column is, and put back after — a user's own
    // global called `acc` outlives this.
    engine.set_global("acc", saved);
    let value = crate::data::lua::from_lua(&accumulator);
    (vec![Record::from_pairs([("reduced", value)])], failure)
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
    let expanded = super::units::expand(expression);
    let source = format!("do {expanded} end");
    let compiled = match engine.load(&source, "each") {
        Ok(compiled) => compiled,
        Err(e) => return Some(expression_error("each", expression, &expanded, e)),
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

    /// A fold over the stream, and `acc` is the running answer.
    #[test]
    fn reduce_folds_the_stream() {
        let rows = vec![
            Record::from_pairs([("n", Val::Int(1))]),
            Record::from_pairs([("n", Val::Int(2))]),
            Record::from_pairs([("n", Val::Int(3))]),
        ];
        let (out, failure) = reduce(&rows, "acc + n", None);
        assert!(failure.is_none(), "{failure:?}");
        assert_eq!(out[0].get("reduced"), Some(&Val::Int(6)));
    }

    /// `--from` decides what kind the fold starts as, so text folds as text.
    #[test]
    fn reduce_starts_where_it_is_told() {
        let rows = vec![
            Record::from_pairs([("s", Val::Str("a".into()))]),
            Record::from_pairs([("s", Val::Str("b".into()))]),
        ];
        let (out, failure) = reduce(&rows, "acc .. s", Some(""));
        assert!(failure.is_none(), "{failure:?}");
        assert_eq!(out[0].get("reduced"), Some(&Val::Str("ab".into())));
    }

    /// **`acc` is borrowed, not taken.** It is bound for the fold exactly as a column is, and a
    /// user's own global of that name outlives the pipeline — the same bug `Bound` exists for.
    #[test]
    fn reduce_puts_back_an_acc_of_its_own() {
        let engine = session_engine();
        engine.set_global("acc", Value::Str("mine".into()));
        let rows = vec![Record::from_pairs([("n", Val::Int(1))])];
        let (_, failure) = reduce(&rows, "acc + n", None);
        assert!(failure.is_none(), "{failure:?}");
        assert_eq!(as_text(&engine.global("acc")).as_deref(), Some("mine"));
    }

    /// An empty stream folds to the value it started from, rather than to nothing.
    #[test]
    fn reducing_nothing_answers_the_start() {
        let (out, failure) = reduce(&[], "acc + n", None);
        assert!(failure.is_none(), "{failure:?}");
        assert_eq!(out[0].get("reduced"), Some(&Val::Int(0)));
    }

    /// **The error names what was typed, not what oslo compiled.**
    ///
    /// An expression is wrapped in `return (…)`, so an unfinished one used to report
    /// `found "RightParen"` — a bracket that appears nowhere in what the user wrote. It also never
    /// said which expression, which in a pipeline holding several is a search rather than a
    /// diagnosis.
    #[test]
    fn a_broken_expression_is_reported_in_the_words_it_was_written_in() {
        let (_, failure) = filter(&[], "size >");
        let message = failure.expect("a syntax error is reported");
        assert!(message.starts_with("where: size >:"), "got {message}");
        assert!(
            message.contains("not finished"),
            "an unfinished expression says so: {message}"
        );
        assert!(
            !message.contains("RightParen"),
            "and does not blame oslo's own bracket: {message}"
        );
    }

    /// **Only when the bracket is provably oslo's.** The wrapper adds exactly one `)`, so a typed
    /// expression that has one of its own makes the two indistinguishable — and then the parser's
    /// own words are the honest answer.
    #[test]
    fn a_typed_bracket_leaves_the_parsers_words_alone() {
        let (_, failure) = filter(&[], "size))");
        let message = failure.expect("a syntax error is reported");
        assert!(message.starts_with("where: size)):"), "got {message}");
        assert!(
            !message.contains("not finished"),
            "this one may genuinely be the user's bracket: {message}"
        );
    }

    /// A unit literal is rewritten before compiling, so the compiled text is not what was typed —
    /// and a parser complaining about text that appears nowhere on screen is unreadable.
    #[test]
    fn a_rewritten_expression_shows_what_it_became() {
        let message = expression_error("where", "size > 1GB &&", "size > 1000000000 &&", "boom");
        assert!(
            message.starts_with("where: size > 1GB &&: boom"),
            "got {message}"
        );
        assert!(
            message.contains("read as `size > 1000000000 &&`"),
            "got {message}"
        );

        // Nothing rewritten, nothing to add.
        let plain = expression_error("where", "a > 1", "a > 1", "boom");
        assert_eq!(plain, "where: a > 1: boom");
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

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

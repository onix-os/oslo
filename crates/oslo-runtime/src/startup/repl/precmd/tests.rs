use super::*;
use oslo_base::value::Table;

fn table(fields: &[(&str, Value)]) -> Option<Value> {
    let mut table = Table::new();
    for (name, value) in fields {
        table.set(Value::str(*name), value.clone());
    }
    Some(Value::table(table))
}

fn line() -> String {
    "echo hi".to_string()
}

#[test]
fn nothing_answered_runs_the_line_as_typed() {
    let answer = read(None, line()).expect("not cancelled");
    assert_eq!(answer.text, "echo hi");
    assert!(answer.record);
}

#[test]
fn a_string_replaces_the_line_and_is_still_recorded() {
    let answer = read(Some(Value::str("echo there")), line()).expect("not cancelled");
    assert_eq!(answer.text, "echo there");
    assert!(answer.record, "replacing a line is not hiding it");
}

#[test]
fn false_cancels() {
    assert!(read(Some(Value::Bool(false)), line()).is_none());
    // `true` is not a cancellation, and never was.
    assert!(read(Some(Value::Bool(true)), line()).is_some());
}

#[test]
fn a_table_can_do_both() {
    let answer = read(
        table(&[
            ("text", Value::str("echo other")),
            ("record", Value::Bool(false)),
        ]),
        line(),
    )
    .expect("not cancelled");
    assert_eq!(answer.text, "echo other");
    assert!(!answer.record);
}

/// **The default that keeps a table from meaning more than it says.**
#[test]
fn a_table_that_mentions_nothing_changes_nothing() {
    let answer = read(table(&[]), line()).expect("not cancelled");
    assert_eq!(answer.text, "echo hi");
    assert!(answer.record, "an absent `record` is not a veto");

    // Nor is any other truthy spelling of it — only `false` suppresses.
    for spelling in [Value::Bool(true), Value::str("yes"), Value::int(1)] {
        let answer = read(table(&[("record", spelling)]), line()).expect("not cancelled");
        assert!(answer.record);
    }
}

#[test]
fn a_table_may_cancel() {
    assert!(read(table(&[("cancel", Value::Bool(true))]), line()).is_none());
    // And an explicit `cancel = false` is not a cancellation.
    assert!(read(table(&[("cancel", Value::Bool(false))]), line()).is_some());
}

#[test]
fn a_veto_alone_leaves_the_line_alone() {
    let answer = read(table(&[("record", Value::Bool(false))]), line()).expect("not cancelled");
    assert_eq!(answer.text, "echo hi", "hiding a line does not rewrite it");
    assert!(!answer.record);
}

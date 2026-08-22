use super::*;

fn a_function() -> Value {
    super::super::util::native("nothing", |_, _| Ok(Vec::new()))
}

fn spec(fields: &[(&str, Value)]) -> Vec<Value> {
    let mut table = Table::new();
    for (name, value) in fields {
        table.set(Value::str(*name), value.clone());
    }
    vec![Value::table(table)]
}

/// **The pair form is gone**, and says so by name rather than complaining about argument #1.
#[test]
fn a_name_and_a_function_are_refused_and_told_the_shape() {
    let refused = declaration(&[Value::str("note"), a_function()]).expect_err("should refuse");
    assert!(refused.to_string().contains("run = function"), "{refused}");
}

#[test]
fn a_table_with_only_the_two_required_fields_is_enough() {
    let read = declaration(&spec(&[
        ("name", Value::str("note")),
        ("run", a_function()),
    ]))
    .expect("read");
    assert_eq!(read.name, "note");
    assert!(read.desc.is_none());
    assert!(read.complete.is_none());
}

#[test]
fn a_table_carries_what_the_pair_could_not() {
    let read = declaration(&spec(&[
        ("name", Value::str("note")),
        ("run", a_function()),
        ("desc", Value::str("write a note down")),
        ("complete", a_function()),
    ]))
    .expect("read");
    assert_eq!(read.name, "note");
    assert_eq!(read.desc.as_deref(), Some("write a note down"));
    assert!(read.complete.is_some());
}

#[test]
fn a_table_may_say_only_what_it_must() {
    let read = declaration(&spec(&[
        ("name", Value::str("note")),
        ("run", a_function()),
    ]))
    .expect("read");
    assert!(read.desc.is_none());
    assert!(read.complete.is_none());
}

#[test]
fn a_declaration_missing_its_name_or_its_body_is_refused() {
    assert!(declaration(&[]).is_err());
    assert!(declaration(&[Value::str("note")]).is_err(), "no function");
    assert!(
        declaration(&spec(&[("run", a_function())])).is_err(),
        "no name"
    );
    assert!(
        declaration(&spec(&[("name", Value::str("note"))])).is_err(),
        "no run"
    );
    // A name that is only spaces is not a name.
    assert!(declaration(&[Value::str("  "), a_function()]).is_err());
}

/// A `complete` that is not callable is a mistake worth naming, not something to ignore.
#[test]
fn a_complete_that_is_not_a_function_is_refused() {
    let refused = declaration(&spec(&[
        ("name", Value::str("note")),
        ("run", a_function()),
        ("complete", Value::str("nope")),
    ]))
    .expect_err("should refuse");
    assert!(refused.to_string().contains("complete"), "{refused}");
}

#[test]
fn the_name_is_trimmed_the_way_it_always_was() {
    let read = declaration(&spec(&[
        ("name", Value::str("  note  ")),
        ("run", a_function()),
    ]))
    .expect("read");
    assert_eq!(read.name, "note");
}

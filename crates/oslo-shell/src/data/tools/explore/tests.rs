use super::*;

fn row(pairs: &[(&str, Val)]) -> Record {
    Record::from_pairs(pairs.iter().map(|(n, v)| (*n, v.clone())))
}

/// Every column any row has, even one only the second row carries — the same union the drawn table
/// takes, so opening a table cannot show fewer columns than printing it did.
#[test]
fn the_columns_are_the_union_of_the_rows() {
    let sheet = sheet(
        "explore",
        &[
            row(&[("a", Val::Int(1))]),
            row(&[("b", Val::Int(2)), ("a", Val::Int(3))]),
        ],
    );
    assert_eq!(sheet.columns, vec!["a", "b"]);
    assert_eq!(
        sheet.rows[0][1],
        Cell::Flat(String::new()),
        "a missing cell"
    );
}

/// The summary on a nested cell is the drawn table's own, so a cell reads the same in both places.
#[test]
fn a_nested_cell_carries_the_drawn_summary() {
    let sheet = sheet(
        "explore",
        &[row(&[
            ("tags", Val::List(vec![Val::Int(1), Val::Int(2)])),
            ("meta", Val::Record(row(&[("k", Val::Str("v".into()))]))),
        ])],
    );
    assert_eq!(sheet.rows[0][0].text(), "<2 items>");
    assert_eq!(sheet.rows[0][1].text(), "<1 field>");
}

/// A record opens as `field`/`value`, which is how a record is read: down, not across.
#[test]
fn a_record_opens_as_field_and_value() {
    let sheet = sheet(
        "explore",
        &[row(&[(
            "meta",
            Val::Record(row(&[
                ("kind", Val::Str("x".into())),
                ("size", Val::Int(10)),
            ])),
        )])],
    );
    let inner = sheet.rows[0][0].sheet().expect("a record opens");
    assert_eq!(inner.title, "meta", "the column names the level");
    assert_eq!(inner.columns, vec!["field", "value"]);
    assert_eq!(inner.rows.len(), 2);
    assert_eq!(inner.rows[0][0].text(), "kind");
    assert_eq!(inner.rows[1][1].text(), "10");
}

/// A list of records opens as the table it already is; a list of anything else as one column.
#[test]
fn a_list_opens_as_a_table_when_it_is_one() {
    let table = sheet(
        "explore",
        &[row(&[(
            "rows",
            Val::List(vec![
                Val::Record(row(&[("n", Val::Int(1))])),
                Val::Record(row(&[("n", Val::Int(2))])),
            ]),
        )])],
    );
    let inner = table.rows[0][0].sheet().expect("a list opens");
    assert_eq!(inner.columns, vec!["n"]);
    assert_eq!(inner.rows.len(), 2);

    let plain = sheet(
        "explore",
        &[row(&[(
            "tags",
            Val::List(vec![Val::Str("p".into()), Val::Str("q".into())]),
        )])],
    );
    let inner = plain.rows[0][0].sheet().expect("a list opens");
    assert_eq!(inner.columns, vec!["value"]);
    assert_eq!(inner.rows.len(), 2);
}

/// An empty list or record is a flat cell: `<0 items>` opening onto a blank level reads as the
/// viewer having broken, not as the cell having been empty.
#[test]
fn an_empty_nesting_is_not_a_door() {
    let sheet = sheet(
        "explore",
        &[row(&[
            ("l", Val::List(Vec::new())),
            ("r", Val::Record(Record::new())),
        ])],
    );
    assert!(sheet.rows[0][0].sheet().is_none(), "{:?}", sheet.rows[0][0]);
    assert!(sheet.rows[0][1].sheet().is_none(), "{:?}", sheet.rows[0][1]);
}

/// The alignment is the drawn table's, at every level — a number that was right-aligned in the
/// transcript must not jump left when the table is opened.
#[test]
fn numeric_columns_are_marked_at_every_level() {
    let sheet = sheet(
        "explore",
        &[
            row(&[("name", Val::Str("a".into())), ("n", Val::Int(9))]),
            row(&[("name", Val::Str("b".into())), ("n", Val::Int(2315))]),
        ],
    );
    assert_eq!(sheet.numeric, vec![false, true]);

    let nested = super::sheet(
        "explore",
        &[row(&[(
            "meta",
            Val::Record(row(&[("size", Val::Int(10))])),
        )])],
    );
    let inner = nested.rows[0][0].sheet().expect("a record opens");
    assert_eq!(
        inner.numeric,
        vec![false, true],
        "field text, value numeric"
    );
}

/// A newline in a cell is folded, because a row that is two lines is not a row — here for the same
/// reason it is in the drawn table, and by the same function.
#[test]
fn a_control_character_is_folded() {
    let sheet = sheet(
        "explore",
        &[row(&[("n", Val::Str("first\nsecond".into()))])],
    );
    assert_eq!(sheet.rows[0][0].text(), "first\\nsecond");
}

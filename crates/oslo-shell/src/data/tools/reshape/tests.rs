use super::*;

fn rows() -> Vec<Record> {
    vec![
        Record::from_pairs([
            ("name", Val::Str("a".into())),
            ("size", Val::Int(10)),
            ("mode", Val::Int(420)),
        ]),
        Record::from_pairs([
            ("name", Val::Str("b".into())),
            ("size", Val::Int(20)),
            ("mode", Val::Int(420)),
        ]),
    ]
}

/// The complement of `cols`, and the surviving columns keep their order.
#[test]
fn reject_drops_named_columns() {
    let out = reject(&rows(), &["mode".to_string()]);
    assert_eq!(out[0].columns(), ["name", "size"]);
    assert_eq!(out.len(), 2);
}

/// A rename keeps the column where it was: a record's order decides what the drawn table shows.
#[test]
fn rename_keeps_the_column_in_place() {
    let out = rename(&rows(), "size", "bytes");
    assert_eq!(out[0].columns(), ["name", "bytes", "mode"]);
    assert_eq!(out[0].get("bytes"), Some(&Val::Int(10)));
    assert_eq!(out[0].get("size"), None);
}

/// A name nothing has is simply not renamed, rather than inventing a column.
#[test]
fn renaming_a_column_that_is_not_there_changes_nothing() {
    let out = rename(&rows(), "nope", "x");
    assert_eq!(out[0].columns(), ["name", "size", "mode"]);
}

/// **`insert` refuses a column that exists and `update` refuses one that does not.** Two of the
/// three refuse, because `insert` on an existing column is nearly always a typo for `update`, and
/// overwriting silently is how a pipeline loses a column without saying so.
#[test]
fn the_three_computing_verbs_differ_only_in_what_they_refuse() {
    let values = vec![Some(Val::Int(1)), Some(Val::Int(2))];

    assert!(
        assign(&rows(), "size", &values, When::Absent).is_err(),
        "insert over a column"
    );
    assert!(
        assign(&rows(), "kb", &values, When::Absent).is_ok(),
        "insert a new column"
    );

    assert!(
        assign(&rows(), "kb", &values, When::Present).is_err(),
        "update a column that is not there"
    );
    assert!(
        assign(&rows(), "size", &values, When::Present).is_ok(),
        "update an existing column"
    );

    assert!(assign(&rows(), "kb", &values, When::Either).is_ok());
    assert!(assign(&rows(), "size", &values, When::Either).is_ok());
}

/// A row whose expression raised is left exactly as it was, rather than being given a hole.
#[test]
fn a_row_that_could_not_be_computed_is_left_alone() {
    let values = vec![Some(Val::Int(1)), None];
    let out = assign(&rows(), "kb", &values, When::Absent).expect("insert");
    assert_eq!(out[0].get("kb"), Some(&Val::Int(1)));
    assert_eq!(out[1].get("kb"), None, "the row that failed is untouched");
    assert_eq!(out[1].get("size"), Some(&Val::Int(20)), "and still whole");
}

/// An empty stream says nothing about which columns exist, so `update` does not refuse it.
#[test]
fn an_empty_stream_refuses_nothing() {
    assert!(assign(&[], "any", &[], When::Present).is_ok());
}

/// A nested record becomes columns named the way a path would have reached them, so the flattened
/// form is not a second vocabulary.
#[test]
fn flatten_names_columns_the_way_a_path_would() {
    let inner = Record::from_pairs([("running", Val::Bool(true)), ("pid", Val::Int(9))]);
    let deeper = Record::from_pairs([("a", Val::Record(Record::from_pairs([("b", Val::Int(1))])))]);
    let rows = vec![Record::from_pairs([
        ("id", Val::Str("x".into())),
        ("state", Val::Record(inner)),
        ("deep", Val::Record(deeper)),
    ])];
    let out = flatten(&rows);
    assert_eq!(
        out[0].columns(),
        ["id", "state.running", "state.pid", "deep.a.b"],
        "recursive, and in the order the record had"
    );
    assert_eq!(out[0].get("state.running"), Some(&Val::Bool(true)));
    // And the name it produced is one `Path` finds again.
    assert_eq!(
        Path::parse("state.running").get(&out[0]).unwrap(),
        Some(&Val::Bool(true))
    );
}

/// A list is left alone: spreading it into columns would make the column set depend on the data,
/// and two rows would stop having the same shape.
#[test]
fn flatten_leaves_a_list_alone() {
    let rows = vec![Record::from_pairs([(
        "tags",
        Val::List(vec![Val::Int(1), Val::Int(2)]),
    )])];
    assert_eq!(flatten(&rows)[0].columns(), ["tags"]);
}

/// The first row becomes the names and stops being a row.
#[test]
fn headers_promotes_the_first_row() {
    let rows = vec![
        Record::from_pairs([
            ("line", Val::Str("NAME".into())),
            ("b", Val::Str("SIZE".into())),
        ]),
        Record::from_pairs([("line", Val::Str("a".into())), ("b", Val::Int(1))]),
    ];
    let out = headers(&rows);
    assert_eq!(out.len(), 1, "the header row is gone");
    assert_eq!(out[0].columns(), ["NAME", "SIZE"]);
    assert_eq!(out[0].get("NAME"), Some(&Val::Str("a".into())));
}

/// An empty stream has no header to promote.
#[test]
fn headers_of_nothing_is_nothing() {
    assert!(headers(&[]).is_empty());
}

/// The ends of a stream, and every nth row.
#[test]
fn skip_and_every_take_from_the_stream() {
    let five: Vec<Record> = (0..5)
        .map(|i| Record::from_pairs([("n", Val::Int(i))]))
        .collect();
    assert_eq!(skip(&five, 2).len(), 3);
    assert_eq!(skip(&five, 99).len(), 0, "more than there is");
    assert_eq!(every(&five, 2).len(), 3, "0, 2 and 4");
    assert_eq!(every(&five, 2)[1].get("n"), Some(&Val::Int(2)));
    assert_eq!(every(&five, 0).len(), 0, "a step of nothing is nothing");
}

/// The index goes first, because it is what you are about to read.
#[test]
fn enumerate_counts_from_zero_and_leads() {
    let out = enumerate(&rows());
    assert_eq!(out[0].columns(), ["index", "name", "size", "mode"]);
    assert_eq!(out[1].get("index"), Some(&Val::Int(1)));
}

/// **An error cell survives `compact`**, because it is something: the cell failed and the row is
/// entitled to say so. Dropping it would hide exactly the rows worth looking at.
#[test]
fn compact_drops_nulls_but_keeps_errors() {
    let rows = vec![
        Record::from_pairs([("free", Val::Int(1))]),
        Record::from_pairs([("free", Val::Null)]),
        Record::from_pairs([("free", Val::Error("stale handle".into()))]),
        Record::from_pairs([("other", Val::Int(2))]),
    ];
    let out = compact(&rows, Some("free"));
    assert_eq!(out.len(), 2, "the null and the missing column go");
    assert_eq!(out[1].get("free"), Some(&Val::Error("stale handle".into())));
}

/// With no column named, a row is kept when none of its cells is null.
#[test]
fn compact_with_no_column_looks_at_them_all() {
    let rows = vec![
        Record::from_pairs([("a", Val::Int(1)), ("b", Val::Int(2))]),
        Record::from_pairs([("a", Val::Int(1)), ("b", Val::Null)]),
    ];
    assert_eq!(compact(&rows, None).len(), 1);
}

/// A default fills an absent or null cell and leaves every other row alone.
#[test]
fn default_fills_only_the_gaps() {
    let rows = vec![
        Record::from_pairs([("a", Val::Int(1))]),
        Record::from_pairs([("a", Val::Null)]),
        Record::from_pairs([("b", Val::Int(3))]),
    ];
    let out = default(&rows, "a", &Val::Str("-".into()));
    assert_eq!(out[0].get("a"), Some(&Val::Int(1)), "untouched");
    assert_eq!(out[1].get("a"), Some(&Val::Str("-".into())), "null filled");
    assert_eq!(
        out[2].get("a"),
        Some(&Val::Str("-".into())),
        "absent filled"
    );
}

use super::*;
use crate::data::Val;

fn left() -> Vec<Record> {
    vec![
        Record::from_pairs([("name", Val::Str("a".into())), ("n", Val::Int(1))]),
        Record::from_pairs([("name", Val::Str("b".into())), ("n", Val::Int(2))]),
        Record::from_pairs([("name", Val::Str("c".into())), ("n", Val::Int(3))]),
    ]
}

fn right() -> Vec<Record> {
    vec![
        Record::from_pairs([
            ("name", Val::Str("a".into())),
            ("role", Val::Str("web".into())),
        ]),
        Record::from_pairs([
            ("name", Val::Str("b".into())),
            ("role", Val::Str("db".into())),
        ]),
        Record::from_pairs([
            ("name", Val::Str("zz".into())),
            ("role", Val::Str("gone".into())),
        ]),
    ]
}

/// **Inner by default**: a left row with no match does not survive, so "did this match?" stays
/// answerable downstream.
#[test]
fn lookup_is_inner_by_default() {
    let out = lookup(&left(), &right(), "name", false);
    assert_eq!(out.len(), 2, "c has no match and does not survive");
    assert_eq!(out[0].get("role"), Some(&Val::Str("web".into())));
    assert_eq!(out[1].get("name"), Some(&Val::Str("b".into())));
}

/// The left-outer form, for when the question is which rows *failed* to match.
#[test]
fn keeping_unmatched_rows_keeps_them_whole() {
    let out = lookup(&left(), &right(), "name", true);
    assert_eq!(out.len(), 3);
    assert_eq!(out[2].get("name"), Some(&Val::Str("c".into())));
    assert_eq!(out[2].get("role"), None, "unmatched, so no right columns");
}

/// **A collision keeps both sides.** Overwriting loses data silently and skipping loses it loudly;
/// a suffix is the only one of the three where nothing disappears.
#[test]
fn a_colliding_column_arrives_suffixed() {
    let right = vec![Record::from_pairs([
        ("name", Val::Str("a".into())),
        ("n", Val::Int(99)),
    ])];
    let out = lookup(&left(), &right, "name", false);
    assert_eq!(
        out[0].get("n"),
        Some(&Val::Int(1)),
        "the left keeps the name"
    );
    assert_eq!(
        out[0].get("n_2"),
        Some(&Val::Int(99)),
        "and the right is kept too"
    );
}

/// The key is the same value on both sides by construction, so it is not carried twice.
#[test]
fn the_key_is_not_duplicated() {
    let out = lookup(&left(), &right(), "name", false);
    assert_eq!(out[0].get("name_2"), None);
    assert_eq!(out[0].columns(), ["name", "n", "role"]);
}

/// Several matches make several rows — that is what a join is, and taking the first would be a
/// different operation wearing this one's name.
#[test]
fn several_matches_make_several_rows() {
    let right = vec![
        Record::from_pairs([
            ("name", Val::Str("a".into())),
            ("role", Val::Str("one".into())),
        ]),
        Record::from_pairs([
            ("name", Val::Str("a".into())),
            ("role", Val::Str("two".into())),
        ]),
    ];
    let out = lookup(&left(), &right, "name", false);
    assert_eq!(out.len(), 2);
    assert_eq!(out[0].get("role"), Some(&Val::Str("one".into())));
    assert_eq!(out[1].get("role"), Some(&Val::Str("two".into())));
}

/// Joining on a value, not on how it is drawn — the rule `group-by` follows for the same reason.
#[test]
fn the_join_is_on_the_value_not_the_rendering() {
    let a = vec![Record::from_pairs([("s", Val::Size(4_200_000_000))])];
    let b = vec![Record::from_pairs([
        ("s", Val::Size(4_200_000_001)),
        ("tag", Val::Str("other".into())),
    ])];
    // Both render as 4.2G but are different byte counts, so they must not match.
    assert!(lookup(&a, &b, "s", false).is_empty());
}

/// A row missing the key joins to nothing rather than to everything.
#[test]
fn a_row_without_the_key_matches_nothing() {
    let a = vec![Record::from_pairs([("other", Val::Int(1))])];
    assert!(lookup(&a, &right(), "name", false).is_empty());
    assert_eq!(
        lookup(&a, &right(), "name", true).len(),
        1,
        "but survives --keep"
    );
}

/// One stream after another, with nothing reconciled: the drawn table takes the union as always.
#[test]
fn append_puts_one_stream_after_the_other() {
    let out = append(&left(), &right());
    assert_eq!(out.len(), 6);
    assert_eq!(out[3].get("role"), Some(&Val::Str("web".into())));
    assert_eq!(
        out[3].get("n"),
        None,
        "a column only one side has is absent"
    );
}

/// Paired by position, and **as long as the left stream** — extra rows on the right have no row to
/// merge into, and inventing one would change how many rows the pipeline has.
#[test]
fn merge_pairs_by_position_and_keeps_the_left_length() {
    let out = merge(&left(), &right());
    assert_eq!(out.len(), 3);
    assert_eq!(out[0].get("role"), Some(&Val::Str("web".into())));
    assert_eq!(
        out[2].get("role"),
        Some(&Val::Str("gone".into())),
        "the third pairs positionally, regardless of name"
    );

    let short = merge(&left(), &right()[..1]);
    assert_eq!(
        short.len(),
        3,
        "a short right side does not shorten the left"
    );
    assert_eq!(short[1].get("role"), None);
}

/// The right side wins a collision in `merge`, which is what "merge into" means.
#[test]
fn merge_lets_the_right_side_win() {
    let right = vec![Record::from_pairs([("n", Val::Int(99))])];
    assert_eq!(merge(&left(), &right)[0].get("n"), Some(&Val::Int(99)));
}

/// Nothing on either side is quiet rather than a panic.
#[test]
fn empty_sides_are_quiet() {
    assert!(lookup(&[], &right(), "name", false).is_empty());
    assert!(lookup(&left(), &[], "name", false).is_empty());
    assert_eq!(lookup(&left(), &[], "name", true).len(), 3);
    assert_eq!(append(&[], &right()).len(), 3);
    assert!(merge(&[], &right()).is_empty());
}

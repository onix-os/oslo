use super::*;

fn rows() -> Vec<Record> {
    vec![
        Record::from_pairs([("user", Val::Str("root".into())), ("size", Val::Size(100))]),
        Record::from_pairs([("user", Val::Str("bres".into())), ("size", Val::Size(200))]),
        Record::from_pairs([("user", Val::Str("root".into())), ("size", Val::Size(300))]),
    ]
}

#[test]
fn group_by_answers_one_row_per_value_in_the_order_first_seen() {
    let grouped = group_by(&rows(), "user");
    assert_eq!(grouped.len(), 2);
    assert_eq!(grouped[0].get("user"), Some(&Val::Str("root".into())));
    assert_eq!(grouped[0].get("count"), Some(&Val::Int(2)));
    assert_eq!(grouped[1].get("user"), Some(&Val::Str("bres".into())));
    // The rows come with it, so a group-by is not a dead end.
    assert!(matches!(grouped[0].get("rows"), Some(Val::List(_))));
}

/// **Grouping is on the value, not on how it is drawn.** A `Size` displays as `4.2G` at two
/// different byte counts, so grouping on the rendering would merge rows that differ.
#[test]
fn two_sizes_that_look_alike_do_not_group_together() {
    let rows = vec![
        Record::from_pairs([("n", Val::Size(4_200_000_000))]),
        Record::from_pairs([("n", Val::Size(4_200_000_001))]),
    ];
    assert_eq!(group_by(&rows, "n").len(), 2);
}

#[test]
fn a_row_without_the_column_is_in_no_group() {
    let mut rows = rows();
    rows.push(Record::from_pairs([("other", Val::Int(1))]));
    let grouped = group_by(&rows, "user");
    assert_eq!(grouped.len(), 2, "no group for the row that has no user");
    let total: i64 = grouped
        .iter()
        .filter_map(|row| match row.get("count") {
            Some(Val::Int(n)) => Some(*n),
            _ => None,
        })
        .sum();
    assert_eq!(total, 3);
}

#[test]
fn count_answers_how_many() {
    assert_eq!(count(&rows())[0].get("count"), Some(&Val::Int(3)));
}

/// **`group-by | count` must answer how many in each group**, not how many groups — the reading
/// that would make the pair useless.
#[test]
fn count_after_group_by_keeps_each_groups_own_number() {
    let counted = count(&group_by(&rows(), "user"));
    assert_eq!(counted.len(), 2);
    assert_eq!(counted[0].get("count"), Some(&Val::Int(2)));
    assert_eq!(counted[1].get("count"), Some(&Val::Int(1)));
    // The rows are dropped: a count is a summary, not a carrier.
    assert_eq!(counted[0].get("rows"), None);
}

#[test]
fn uniq_keeps_the_first_of_each() {
    let by_user = uniq(&rows(), Some("user"));
    assert_eq!(by_user.len(), 2);
    assert_eq!(by_user[0].get("size"), Some(&Val::Size(100)), "the first");

    // With no field, a whole row must match to be a duplicate.
    let mut doubled = rows();
    doubled.push(rows()[0].clone());
    assert_eq!(uniq(&doubled, None).len(), 3);
}

#[test]
fn stats_reports_the_shape_of_a_numeric_column() {
    let summary = &stats(&rows(), "size")[0];
    assert_eq!(summary.get("count"), Some(&Val::Int(3)));
    assert_eq!(summary.get("min"), Some(&Val::Int(100)));
    assert_eq!(summary.get("max"), Some(&Val::Int(300)));
    assert_eq!(summary.get("sum"), Some(&Val::Int(600)));
    assert_eq!(summary.get("mean"), Some(&Val::Float(200.0)));
}

/// A column that is text in some rows is not an error, and `count` says how many took part — so a
/// mean over two of five rows can be seen for what it is.
#[test]
fn stats_counts_only_the_cells_that_are_numbers() {
    let rows = vec![
        Record::from_pairs([("n", Val::Int(2))]),
        Record::from_pairs([("n", Val::Str("four".into()))]),
        // Text that *is* a number counts: `parse` and `from json` produce strings.
        Record::from_pairs([("n", Val::Str(" 4 ".into()))]),
    ];
    let summary = &stats(&rows, "n")[0];
    assert_eq!(summary.get("count"), Some(&Val::Int(2)));
    assert_eq!(summary.get("sum"), Some(&Val::Int(6)));
}

/// **Where aggregation usually gets it wrong.** Nothing here may divide by zero, panic, or claim a
/// minimum of infinity.
#[test]
fn an_empty_stream_summarises_to_something_honest() {
    let none: Vec<Record> = Vec::new();
    assert!(group_by(&none, "user").is_empty());
    assert!(uniq(&none, None).is_empty());
    assert_eq!(count(&none)[0].get("count"), Some(&Val::Int(0)));

    let summary = &stats(&none, "size")[0];
    assert_eq!(summary.get("count"), Some(&Val::Int(0)));
    assert_eq!(summary.get("mean"), None, "no rows, no mean");
    assert_eq!(summary.get("min"), None);
}

#[test]
fn one_row_is_its_own_summary() {
    let one = vec![Record::from_pairs([("n", Val::Int(7))])];
    assert_eq!(count(&one)[0].get("count"), Some(&Val::Int(1)));
    assert_eq!(uniq(&one, None).len(), 1);
    let summary = &stats(&one, "n")[0];
    assert_eq!(summary.get("min"), summary.get("max"));
    assert_eq!(summary.get("mean"), Some(&Val::Float(7.0)));
}

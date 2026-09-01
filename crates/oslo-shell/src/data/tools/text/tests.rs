use super::*;

fn words(line: &str) -> Vec<String> {
    line.split(' ').map(str::to_string).collect()
}

fn out(line: &str) -> Vec<String> {
    let (status, rows) = run(&words(line), None, None).expect("text always answers");
    assert_eq!(status, 0, "{line}");
    rows.expect("a run that succeeded produced rows")
        .iter()
        .map(string_of)
        .collect()
}

fn refused(line: &str) -> i32 {
    run(&words(line), None, None)
        .expect("text always answers")
        .0
}

/// The three sources, in the order that decides which one is used.
#[test]
fn operands_win_over_rows_and_rows_over_bytes() {
    let rows = vec![row(Val::Str("from-a-row".into()))];
    let both = run(&words("text upper"), Some(&rows), Some("from-bytes")).unwrap();
    assert_eq!(
        both.1.unwrap().first().map(string_of).unwrap(),
        "FROM-A-ROW"
    );

    let only_bytes = run(&words("text upper"), None, Some("from-bytes")).unwrap();
    assert_eq!(
        only_bytes.1.unwrap().first().map(string_of).unwrap(),
        "FROM-BYTES"
    );

    assert_eq!(out("text upper spelled-out"), ["SPELLED-OUT"]);
}

/// A stage after `lines` has a `line` column and no `text` one; it must still work unnamed.
#[test]
fn a_row_from_lines_is_read_without_naming_its_column() {
    let rows = vec![Record::from_pairs([("line", Val::Str("hi".into()))])];
    let answered = run(&words("text upper"), Some(&rows), None).unwrap();
    assert_eq!(answered.1.unwrap().first().map(string_of).unwrap(), "HI");
}

/// Several values out of one is the reason this is a verb rather than a builtin.
#[test]
fn split_makes_a_row_per_field() {
    assert_eq!(out("text split : a:b:c"), ["a", "b", "c"]);
    assert_eq!(out("text split -- : a:b:c"), ["a", "b", "c"]);
    assert_eq!(out("text split --max 1 = a=b=c"), ["a", "b=c"]);
    assert_eq!(out("text split  abc"), ["a", "b", "c"]);
}

#[test]
fn join_and_collect_answer_one_row() {
    assert_eq!(out("text join , x y z"), ["x,y,z"]);
    assert_eq!(out("text collect a b"), ["a\nb"]);
}

#[test]
fn match_is_a_substring_until_it_is_told_otherwise() {
    assert_eq!(
        out("text match a alpha beta gamma"),
        ["alpha", "beta", "gamma"]
    );
    assert_eq!(out("text match ph alpha beta"), ["alpha"]);
    assert_eq!(out("text match --regex ^a alpha beta"), ["alpha"]);
    assert_eq!(out("text match --invert ph alpha beta"), ["beta"]);
    assert_eq!(out("text match --ignore-case AL alpha"), ["alpha"]);
}

/// The first occurrence, like `${x/a/b}` — the two spellings must not disagree.
#[test]
fn replace_changes_one_unless_told_all() {
    assert_eq!(out("text replace o 0 foo"), ["f0o"]);
    assert_eq!(out("text replace --all o 0 foo"), ["f00"]);
    assert_eq!(out("text replace --regex o+ 0 foo"), ["f0"]);
}

#[test]
fn trim_takes_both_ends_unless_one_is_named() {
    assert_eq!(out("text trim --chars x xxaxx"), ["a"]);
    assert_eq!(out("text trim --left --chars x xxaxx"), ["axx"]);
    assert_eq!(out("text trim --right --chars x xxaxx"), ["xxa"]);
}

/// Counted from 1, and a negative start counts back from the end.
#[test]
fn sub_counts_characters_from_one() {
    assert_eq!(out("text sub --start 2 --length 3 abcdef"), ["bcd"]);
    assert_eq!(out("text sub --start -3 abcdef"), ["def"]);
    assert_eq!(out("text sub --start 99 abc"), [""]);
    assert_eq!(refused("text sub --start 0 abc"), 2);
}

/// Padding and truncating are different requests, so a wide string is left alone.
#[test]
fn pad_never_shortens() {
    assert_eq!(out("text pad --width 4 --char . x"), ["...x"]);
    assert_eq!(out("text pad --width 4 --right --char . x"), ["x..."]);
    assert_eq!(out("text pad --width 2 abcd"), ["abcd"]);
}

#[test]
fn length_counts_characters_rather_than_bytes() {
    assert_eq!(out("text length héllo"), ["5"]);
    assert_eq!(out("text sub --start 2 --length 1 héllo"), ["é"]);
}

/// What `escape` writes, the shell reads back as the same characters — including a quote.
#[test]
fn escape_and_unescape_round_trip() {
    assert_eq!(out("text escape it's"), ["'it'\\''s'"]);
    let escaped = out("text escape it's").remove(0);
    assert_eq!(unescape_one(&escaped), "it's");
}

#[test]
fn a_subcommand_that_is_not_one_is_refused() {
    assert_eq!(refused("text nope x"), 2);
    assert_eq!(refused("text"), 2);
    assert_eq!(refused("text split"), 2);
    assert_eq!(refused("text pad x"), 2);
    assert_eq!(refused("text repeat --count -1 x"), 2);
    assert_eq!(refused("text upper --nope x"), 2);
    assert_eq!(refused("text pad --width wide x"), 2);
}

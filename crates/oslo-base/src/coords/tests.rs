//! What a coordinate reads, and what it refuses to read.

use super::*;

const HOSTS: &str =
    "web-01  10.0.0.1  nginx\nweb-02  10.0.0.2  apache\ndb-01   10.0.0.9  postgres\n";

#[track_caller]
fn reads(coordinate: &str, wanted: &[&str]) {
    let coord = parse(coordinate).unwrap_or_else(|| panic!("{coordinate:?} did not parse"));
    assert_eq!(select(&coord, HOSTS), wanted, "for {{{coordinate}}}");
}

/// **How many dimensions you write is what says which is which.** Nothing is marked, so this is the
/// whole of the disambiguation and it is worth pinning first.
#[test]
fn the_count_of_dimensions_decides_their_meaning() {
    assert_eq!(
        parse("2"),
        Some(Coord {
            subject: Subject::Output,
            stream: Sel::At(0),
            line: Sel::At(2),
            word: None
        })
    );
    assert_eq!(
        parse("0:1"),
        Some(Coord {
            subject: Subject::Output,
            stream: Sel::At(0),
            line: Sel::At(0),
            word: Some(Sel::At(1))
        })
    );
    assert_eq!(
        parse("1:0:1"),
        Some(Coord {
            subject: Subject::Output,
            stream: Sel::At(1),
            line: Sel::At(0),
            word: Some(Sel::At(1))
        })
    );
    // Four is not a coordinate.
    assert_eq!(parse("1:2:3:4"), None);
}

/// A line is a whole line, and a word is a word.
#[test]
fn lines_and_words() {
    reads("0", &["web-01  10.0.0.1  nginx"]);
    reads("0:0", &["web-01"]);
    reads("0:1", &["10.0.0.1"]);
    reads("1:2", &["apache"]);
}

/// **An absent word is not every word.** `{0}` is one value with its spaces intact — which is the
/// difference between a filename and two filenames.
#[test]
fn an_absent_word_is_the_whole_line() {
    reads("0", &["web-01  10.0.0.1  nginx"]);
    reads("0:*", &["web-01", "10.0.0.1", "nginx"]);
    // An empty word dimension reads as absent, so `{0:}` is the whole line too.
    reads("0:", &["web-01  10.0.0.1  nginx"]);
}

/// Negative counts from the end, in both dimensions.
#[test]
fn negatives_count_from_the_end() {
    reads("-1", &["db-01   10.0.0.9  postgres"]);
    reads("-1:-1", &["postgres"]);
    reads("0:-1", &["nginx"]);
    reads("-2:0", &["web-02"]);
}

/// **Ranges include both ends**, unlike Python and like the brace expansion sitting next to them.
#[test]
fn ranges_include_both_ends() {
    reads(
        "0..1:",
        &["web-01  10.0.0.1  nginx", "web-02  10.0.0.2  apache"],
    );
    reads("0:0..1", &["web-01", "10.0.0.1"]);
    reads("..1:0", &["web-01", "web-02"]);
    reads("1..:0", &["web-02", "db-01"]);
    reads("*:0", &["web-01", "web-02", "db-01"]);
    reads("0..-1:0", &["web-01", "web-02", "db-01"]);
}

/// Out of range is empty, never an error — input is ragged and refusing to run is worse than a
/// blank.
#[test]
fn out_of_range_is_empty() {
    reads("9", &[]);
    reads("0:9", &[]);
    reads("9:9", &[]);
    // A backwards range selects nothing rather than reversing.
    reads("2..0:", &[]);
    // And an empty stream answers nothing rather than panicking on the arithmetic.
    let coord = parse("-1:-1").expect("parses");
    assert_eq!(select(&coord, ""), Vec::<String>::new());
    assert_eq!(select(&coord, "\n"), Vec::<String>::new());
}

/// **A trailing newline ends the last line, it does not begin an empty one.** Command output almost
/// always ends in one, so without this every `{-1}` would answer with the empty string.
#[test]
fn a_trailing_newline_does_not_add_a_line() {
    let coord = parse("-1").expect("parses");
    assert_eq!(select(&coord, "a\nb\n"), vec!["b"]);
    assert_eq!(select(&coord, "a\nb"), vec!["b"]);
    // Two trailing newlines *do* mean a final empty line, because the second one is content.
    assert_eq!(select(&coord, "a\nb\n\n"), vec![""]);
}

/// **What is not a coordinate must not be read as one**, or an ordinary brace group stops working.
#[test]
fn a_brace_group_that_is_not_a_coordinate_is_refused() {
    for text in [
        "a", "a,b", "1,2", "0.5", "1.5:2", "x:y", "--", "-", "1:2:3:4",
    ] {
        assert_eq!(parse(text), None, "{text:?} should not be a coordinate");
    }
    // `{a..b}` is brace expansion over letters and must stay that way.
    assert_eq!(parse("a..e"), None);
    // But a numeric range with a word dimension is ours — `{0..2}` alone never reaches here,
    // because brace expansion claims it first.
    assert!(parse("0..2:").is_some());
    assert!(parse("0..2:1").is_some());
}

/// The stream dimension is read but not applied here — choosing which text to hand over is the
/// caller's job, and `select` only ever sees one stream.
#[test]
fn the_stream_dimension_is_parsed_for_the_caller() {
    assert_eq!(parse("2:0:0").map(|c| c.stream), Some(Sel::At(2)));
    assert_eq!(parse("0:0").map(|c| c.stream), Some(Sel::At(0)));
    // An empty stream dimension is this command's own input.
    assert_eq!(parse(":0:0").map(|c| c.stream), Some(Sel::At(0)));
}

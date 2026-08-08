//! The encodings, and what an older store's rows still mean.

use super::*;

fn a_directory() -> DirRow {
    DirRow {
        path: "/w/Alpha".to_string(),
        base: "alpha".to_string(),
        root: Some("/w".to_string()),
        visits: 7,
        last_visit: 1_700_000_000,
        dwell_ms: 91_000,
        missing_since: None,
    }
}

#[test]
fn a_directory_reads_back_as_the_row_it_was_written_from() {
    let row = a_directory();
    assert_eq!(DirRow::decode(&row.encode()), Some(row));
}

/// The two nulls, which SQL spelled and this has to encode. Neither may come back as a zero,
/// because a `missing_since` of zero is January 1970 and a `root` of `""` is a directory named
/// nothing.
#[test]
fn a_null_column_reads_back_as_absent_rather_than_as_zero() {
    let mut row = a_directory();
    row.root = None;
    row.missing_since = None;
    let back = DirRow::decode(&row.encode()).expect("it decodes");
    assert_eq!(back.root, None, "outside a repository, not the root itself");
    assert_eq!(back.missing_since, None, "present, not missing since 1970");

    row.missing_since = Some(0);
    assert_eq!(
        DirRow::decode(&row.encode())
            .expect("it decodes")
            .missing_since,
        Some(0),
        "and a real zero is still a real zero"
    );

    let never = RunRow::first("cargo".to_string(), None, 5, 1);
    let back = RunRow::decode(&never.encode()).expect("it decodes");
    assert_eq!(
        back.last_status, None,
        "an exit nobody saw is not an exit 0"
    );
    assert_eq!(back.fails, 0, "and it is not a failure either");
}

/// **A row written before sessions and hosts existed still decodes.**
///
/// The two fields are appended, and a missing trailing field reads as empty rather than
/// failing the whole row — otherwise upgrading oslo would silently discard a history somebody
/// has been building for months.
#[test]
fn a_row_without_the_newer_fields_still_decodes() {
    // Exactly what the old encoder produced: six numbers and the head, and nothing after.
    let old = Key::with_capacity(48)
        .signed(7)
        .signed(1)
        .signed(1_700_000_000)
        .signed(0)
        .signed(250)
        .signed(90)
        .text("cargo")
        .done();
    let back = RunRow::decode(&old).expect("an old row must still decode");
    assert_eq!(back.runs, 7);
    assert_eq!(back.head, "cargo");
    assert!(back.session.is_empty(), "no session to claim");
    assert!(back.host.is_empty());
}

/// And a row written now round trips both.
#[test]
fn the_newer_fields_round_trip() {
    let mut row = RunRow::first("cargo".to_string(), Some(0), 5, 1);
    row.session = "1234-1700000000".to_string();
    row.host = "tron".to_string();
    let back = RunRow::decode(&row.encode()).expect("it decodes");
    assert_eq!(back.session, "1234-1700000000");
    assert_eq!(back.host, "tron");
}

/// Contract item 3: repeats are the entire point and cost nothing after the first.
#[test]
fn absorbing_a_run_adds_the_counters_and_keeps_the_worst_time() {
    let mut row = RunRow::first("cargo build".to_string(), Some(0), 100, 30);
    row.absorb(&RunRow::first("cargo build".to_string(), Some(1), 200, 90));
    row.absorb(&RunRow::first("cargo build".to_string(), Some(0), 300, 10));

    assert_eq!(row.runs, 3);
    assert_eq!(row.fails, 1);
    assert_eq!(row.last_at, 300, "the newest wins");
    assert_eq!(row.last_status, Some(0), "and so does its status");
    assert_eq!(row.total_ms, 130, "time adds up");
    assert_eq!(row.max_ms, 90, "and the worst of it is remembered");
    assert_eq!(RunRow::decode(&row.encode()), Some(row));
}

/// A line that has only ever failed is not a suggestion, and a line whose exit was never
/// observed is not a success.
#[test]
fn a_line_that_never_worked_is_not_one_to_offer_back() {
    let mut failed = RunRow::first("carg".to_string(), Some(127), 1, 1);
    assert!(!failed.worked());
    failed.absorb(&RunRow::first("carg".to_string(), Some(0), 2, 1));
    assert!(failed.worked(), "it works now, so it is a command again");

    let unseen = RunRow::first("sleep".to_string(), None, 1, 1);
    assert!(
        unseen.worked(),
        "never seen to fail either — one run, no failures"
    );
}

/// The keys the contract is written in, decoded back out of themselves. An index row has no
/// value, so if this is wrong the scan has nothing else to fall back on.
#[test]
fn an_index_key_carries_everything_a_scan_needs_to_read_off_it() {
    assert_eq!(field::trailing_id(&key::by_base("rust", 42)), Some(42));
    assert_eq!(field::trailing_id(&key::by_root("/w/beta", 9)), Some(9));
    assert_eq!(
        field::argv_of_run(&key::run(7, "sh", "cargo test")).as_deref(),
        Some("cargo test")
    );
    let rotated = key::by_argv("sh", "cargo test", 7);
    let (argv, dir) = field::argv_and_dir(&rotated).expect("it decodes");
    assert_eq!((argv.as_ref(), dir), ("cargo test", 7));

    // Bytes from somewhere else decode to nothing rather than to a plausible id.
    assert_eq!(field::trailing_id(b"not a key"), None);
    assert_eq!(field::argv_of_run(b"short"), None);
}

/// The ranges, against the keys they have to hold and the neighbours they must not. This is
/// contract item 1's boundary in the smallest form it can be stated in.
#[test]
fn a_range_holds_its_own_rows_and_stops_at_its_neighbours() {
    let here = span::runs_like(7, "sh", "cargo te");
    assert!(here.holds(&key::run(7, "sh", "cargo test")));
    assert!(here.holds(&key::run(7, "sh", "cargo te")));
    assert!(!here.holds(&key::run(7, "sh", "cargo t")));
    assert!(!here.holds(&key::run(7, "sh", "cargo tf")));
    assert!(!here.holds(&key::run(7, "lua", "cargo test")));
    assert!(!here.holds(&key::run(6, "sh", "cargo test")));
    assert!(!here.holds(&key::run(8, "sh", "cargo test")));

    // The cascade's range: one directory, every language, every line.
    let all = span::runs_of(7);
    assert!(all.holds(&key::run(7, "lua", "print(1)")));
    assert!(!all.holds(&key::run(8, "sh", "cargo test")));

    // Naming a directory: the exact name and every extension of it, and nothing that merely
    // contains it — `prust` is a weaker tier and a different question.
    let named = span::bases_like("rust");
    assert!(named.holds(&key::by_base("rust", 1)));
    assert!(named.holds(&key::by_base("rustlings", 1)));
    assert!(!named.holds(&key::by_base("prust", 1)));
    assert!(!named.holds(&key::by_base("rus", 1)));

    // One worktree, and not the one whose path extends it.
    let inside = span::dirs_of_root("/w/beta");
    assert!(inside.holds(&key::by_root("/w/beta", 3)));
    assert!(!inside.holds(&key::by_root("/w/beta-old", 3)));

    let anywhere = span::argv_like("sh", "cargo te");
    assert!(anywhere.holds(&key::by_argv("sh", "cargo test", 99)));
    assert!(!anywhere.holds(&key::by_argv("lua", "cargo test", 99)));
    assert!(!anywhere.holds(&key::by_argv("sh", "cargo t", 99)));
}

/// An empty prefix is every line in the directory rather than none, which is what the cap and
/// the counter both ask for.
#[test]
fn an_empty_prefix_names_the_whole_directory() {
    let span = span::runs_like(7, "sh", "");
    assert!(span.holds(&key::run(7, "sh", "")));
    assert!(span.holds(&key::run(7, "sh", "anything at all")));
    assert!(!span.holds(&key::run(7, "lua", "anything at all")));
}

/// A row this module did not write is refused rather than decoded into something plausible —
/// which is the same rule the seam applies to the file as a whole.
#[test]
fn a_row_from_somewhere_else_decodes_to_nothing() {
    assert_eq!(DirRow::decode(b""), None);
    assert_eq!(DirRow::decode(b"far too short"), None);
    assert_eq!(RunRow::decode(&a_directory().encode()), None);
    // Enough integers, no text: the head was never there.
    assert_eq!(RunRow::decode(&[0u8; 48]), None);
}

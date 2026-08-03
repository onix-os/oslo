//! The frame, at a fixed width with no terminal attached.
//!
//! Every assertion here is about *layout* rather than colour: the escapes are the theme's and
//! change with it, but a row that does not fit its width, or a list that does not sit against the
//! search bar, is wrong under every theme.

use super::*;
use crate::interactive::finder::rank::Ranked;
use crate::track::history::Command;

fn ranked(line: &str, runs: i64, last_at: i64, dir: &str, here: bool) -> Ranked {
    Ranked {
        command: Command {
            line: line.to_string(),
            mode: "sh".to_string(),
            runs,
            last_at,
            dir: dir.to_string(),
            places: 1,
            worked: true,
        },
        score: 0,
        here,
    }
}

fn frame_of<'a>(matches: &'a [Ranked], query: &'a str, rows: usize) -> String {
    frame(&Frame {
        matches,
        selected: 0,
        offset: 0,
        query,
        total: matches.len(),
        cols: 80,
        rows,
        now: 1_000_000_000,
        home: "/home/me",
    })
}

/// Escapes stripped, so a test can assert on what a person would see.
fn plain(rendered: &str) -> String {
    let mut out = String::new();
    let mut chars = rendered.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            out.push(c);
            continue;
        }
        if chars.peek() == Some(&'[') {
            chars.next();
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        }
    }
    out
}

#[test]
fn a_command_appears_with_its_count_age_and_directory() {
    let matches = [ranked(
        "cargo build",
        41,
        999_990_000,
        "/home/me/src",
        false,
    )];
    let seen = plain(&frame_of(&matches, "", 10));
    assert!(seen.contains("cargo build"), "{seen:?}");
    assert!(seen.contains("41×"), "the run count is missing: {seen:?}");
    // 10,000 seconds ago is under a day.
    assert!(seen.contains("2h"), "the age is missing: {seen:?}");
    assert!(seen.contains("~/src"), "the directory is missing: {seen:?}");
}

/// `$HOME` is the least informative part of every path in a narrow column.
#[test]
fn home_is_written_as_a_tilde() {
    assert_eq!(shorten("/home/me", "/home/me"), "~");
    assert_eq!(shorten("/home/me/src/oslo", "/home/me"), "~/src/oslo");
    // Only on a boundary, or `/home/mel` becomes `~l`.
    assert_eq!(shorten("/home/mel", "/home/me"), "/home/mel");
    assert_eq!(shorten("/etc", "/home/me"), "/etc");
    assert_eq!(shorten("/home/me", ""), "/home/me");
}

/// The search bar is the last row and carries the query and the match count.
#[test]
fn the_search_bar_is_at_the_bottom() {
    let matches = [ranked("one", 1, 999_999_999, "/home/me", false)];
    let rendered = frame_of(&matches, "on", 10);
    let last = plain(&rendered)
        .lines()
        .last()
        .unwrap_or_default()
        .to_string();
    assert!(last.contains('❯'), "no prompt on the last row: {last:?}");
    assert!(last.contains("on"), "the query is missing: {last:?}");
    assert!(last.contains("1/1"), "the counter is missing: {last:?}");
}

/// The list grows upward: with one match and room for many, the match sits directly above the
/// separator and the empty rows are at the top.
#[test]
fn the_list_grows_upward_from_the_bar() {
    let matches = [ranked("only", 1, 999_999_999, "/home/me", false)];
    let seen = plain(&frame_of(&matches, "", 8));
    let lines: Vec<&str> = seen.lines().collect();
    // Rows: 8 total, 2 chrome, 6 for the list — five blank then the match.
    let match_row = lines
        .iter()
        .position(|l| l.contains("only"))
        .expect("the match is drawn");
    let separator_row = lines
        .iter()
        .position(|l| l.contains('─'))
        .expect("the separator is drawn");
    assert_eq!(
        match_row + 1,
        separator_row,
        "the match should sit against the separator: {lines:?}"
    );
    assert!(
        lines[..match_row].iter().all(|l| l.trim().is_empty()),
        "the empty rows should be above: {lines:?}"
    );
}

/// No row may exceed the terminal width, or the terminal wraps it and every row below is one
/// line out of place for the rest of the session.
#[test]
fn no_row_is_wider_than_the_terminal() {
    let long = "cargo build --release --target x86_64-unknown-linux-musl --features everything";
    let matches = [
        ranked(
            long,
            999_999,
            1,
            "/home/me/a/very/long/path/that/keeps/going/on",
            true,
        ),
        ranked("short", 1, 999_999_999, "/", false),
    ];
    for row in plain(&frame_of(&matches, "", 10)).lines() {
        assert!(
            printed_width(row) <= 80,
            "row is {} cells: {row:?}",
            printed_width(row)
        );
    }
}

/// A terminal too short for any list still draws the bar rather than dividing by zero.
#[test]
fn a_tiny_terminal_still_renders() {
    let matches = [ranked("one", 1, 1, "/", false)];
    for rows in [1, 2, 3] {
        let rendered = frame_of(&matches, "q", rows);
        assert!(plain(&rendered).contains('❯'), "rows={rows}");
    }
}

/// The selected row is marked, and it is the only one.
#[test]
fn exactly_one_row_carries_the_marker() {
    let matches = [
        ranked("first", 1, 3, "/", false),
        ranked("second", 1, 2, "/", false),
        ranked("third", 1, 1, "/", false),
    ];
    let rendered = frame(&Frame {
        matches: &matches,
        selected: 1,
        offset: 0,
        query: "",
        total: 3,
        cols: 80,
        rows: 10,
        now: 100,
        home: "/home/me",
    });
    let marked: Vec<&str> = plain(&rendered)
        .lines()
        .filter(|l| l.contains('▌'))
        .map(|_| "x")
        .collect();
    assert_eq!(marked.len(), 1, "exactly one marker");
    let row = plain(&rendered)
        .lines()
        .find(|l| l.contains('▌'))
        .unwrap_or_default()
        .to_string();
    assert!(row.contains("second"), "the wrong row is marked: {row:?}");
}

/// With a full window, the *best* match is the row against the search bar and the oldest is at
/// the top. Drawing them the other way round puts the row you are most likely to want at the far
/// end of the screen from the cursor.
#[test]
fn the_best_match_sits_nearest_the_bar() {
    let matches = [
        ranked("best", 1, 5, "/home/me", false),
        ranked("second", 1, 4, "/home/me", false),
        ranked("third", 1, 3, "/home/me", false),
    ];
    let seen = plain(&frame_of(&matches, "", 6));
    let lines: Vec<&str> = seen.lines().collect();
    let row_of = |needle: &str| {
        lines
            .iter()
            .position(|l| l.contains(needle))
            .unwrap_or_else(|| panic!("{needle} is not drawn: {lines:?}"))
    };
    let separator = lines
        .iter()
        .position(|l| l.contains('─'))
        .expect("the separator is drawn");
    assert!(
        row_of("best") < separator,
        "the best match is above the bar"
    );
    assert_eq!(
        row_of("best") + 1,
        separator,
        "the best match should be the row against the separator: {lines:?}"
    );
    assert!(
        row_of("third") < row_of("second") && row_of("second") < row_of("best"),
        "older matches should be further up: {lines:?}"
    );
}

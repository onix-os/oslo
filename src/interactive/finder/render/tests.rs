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
            session: String::new(),
            host: String::new(),
            root: None,
        },
        score: 0,
        here,
    }
}

fn frame_of<'a>(matches: &'a [Ranked], query: &'a str, rows: usize) -> String {
    // The escapes asserted on below are 256-colour ones, and the depth is process-wide — see
    // `theme::held_at`. Held only across the render: the string it returns is already decided.
    let _held = crate::interactive::theme::held_at(crate::interactive::theme::Depth::Ansi256);
    frame(&Frame {
        matches,
        selected: 0,
        offset: 0,
        query,
        scope: Scope::Global,
        total: matches.len(),
        cols: 80,
        rows,
        now: 1_000_000_000,
        // The scanner's frame. Fixed, so a test never depends on when it ran.
        elapsed_ms: 0,
        // Not asking anything: these cover the ordinary search bar.
        confirm: None,
        profile: "default",
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

/// When and how often, then the command. The directory is deliberately absent: it is still the
/// third ranking signal but it is the column you look at least, and it was costing a quarter of
/// the width.
#[test]
fn a_row_reads_when_then_how_often_then_the_command() {
    let matches = [ranked(
        "cargo build",
        41,
        999_990_000,
        "/home/me/src",
        false,
    )];
    let seen = plain(&frame_of(&matches, "", 10));
    let row = seen
        .lines()
        .find(|l| l.contains("cargo build"))
        .expect("the command is drawn");
    let when = row.find("2h").expect("the age is missing");
    let runs = row.find("41").expect("the run count is missing");
    let line = row.find("cargo build").expect("unreachable");
    assert!(when < runs && runs < line, "columns out of order: {row:?}");
    assert!(
        !seen.contains("~/src") && !seen.contains("/home/me"),
        "the directory should not be shown: {seen:?}"
    );
}

/// The input is a three-row surface at the bottom: a blank row, the query, a blank row. The query
/// is therefore the second-from-last row, not the last.
#[test]
fn the_search_bar_sits_inside_its_surface() {
    let matches = [ranked("one", 1, 999_999_999, "/home/me", false)];
    let seen = plain(&frame_of(&matches, "on", 10));
    let lines: Vec<&str> = seen.lines().collect();
    // From the bottom: the margin produces no line, so the surface is the last three.
    let query_row = lines.len() - 2;
    // The scanner stands where the `❯` used to, so the query row is the one carrying it.
    assert!(
        lines[query_row].contains('■') || lines[query_row].contains('⬝'),
        "no scanner on the query row: {:?}",
        lines[query_row]
    );
    assert!(lines[query_row].contains("on"), "{:?}", lines[query_row]);
    assert!(lines[query_row].contains("1/1"), "{:?}", lines[query_row]);
    assert!(
        lines[query_row - 1].trim().is_empty() && lines[query_row + 1].trim().is_empty(),
        "the query should be padded above and below: {lines:?}"
    );
}

/// The tint reaches both terminal edges. A raw unpainted space before or after the styled row is
/// the side margin this layout deliberately does not have.
#[test]
fn the_input_surface_is_full_width() {
    let matches = [ranked("one", 1, 999_999_999, "/home/me", false)];
    let rendered = frame_of(&matches, "needle", 10);
    let row = rendered
        .lines()
        .find(|line| line.contains("needle"))
        .expect("the query row is drawn");
    assert!(!row.starts_with("\x1b[2K "), "left margin remains: {row:?}");
    assert!(!row.ends_with(" \r"), "right margin remains: {row:?}");
}

#[test]
fn the_scope_is_shown_at_the_end_of_the_search_bar() {
    let matches = [ranked("one", 1, 999_999_999, "/home/me", false)];
    let global = plain(&frame_of(&matches, "", 10));
    // `profile @ [scope] || matches/total`.
    assert!(
        global
            .lines()
            .any(|line| line.contains("default @ [global] || 1/1")),
        "{global:?}"
    );

    let local = plain(&frame(&Frame {
        matches: &matches,
        selected: 0,
        offset: 0,
        query: "",
        scope: Scope::Directory,
        total: 1,
        cols: 80,
        rows: 10,
        now: 1_000_000_000,
        // The scanner's frame. Fixed, so a test never depends on when it ran.
        elapsed_ms: 0,
        // Not asking anything: these cover the ordinary search bar.
        confirm: None,
        profile: "default",
    }));
    assert!(
        local
            .lines()
            .any(|line| line.contains("[directory] || 1/1"))
    );
}

#[test]
fn the_scope_badge_uses_accent_on_zero() {
    let matches = [ranked("one", 1, 999_999_999, "/home/me", false)];
    let f = Frame {
        matches: &matches,
        selected: 0,
        offset: 0,
        query: "",
        scope: Scope::Global,
        total: 1,
        cols: 80,
        rows: 10,
        now: 1_000_000_000,
        // The scanner's frame. Fixed, so a test never depends on when it ran.
        elapsed_ms: 0,
        // Not asking anything: these cover the ordinary search bar.
        confirm: None,
        profile: "default",
    };
    let pager = theme::Pager::default();
    let bar = search_bar(&f, &pager, pager.bg, 80, Depth::Ansi256);
    assert!(
        bar.contains("\x1b[38;5;0;48;5;1m[global]\x1b[0m"),
        "scope badge has the wrong colours: {bar:?}"
    );
}

/// The list grows upward: with one match and room for many, the match sits above the one plain
/// separator row and the empty result rows are at the top.
#[test]
fn the_list_grows_upward_from_the_bar() {
    let matches = [ranked("only", 1, 999_999_999, "/home/me", false)];
    let seen = plain(&frame_of(&matches, "", 8));
    let lines: Vec<&str> = seen.lines().collect();
    let match_row = lines
        .iter()
        .position(|l| l.contains("only"))
        .expect("the match is drawn");
    // 8 rows: 3 for the surface, 2 margins, 3 for the list. The match is the last list row.
    assert_eq!(
        match_row, 2,
        "the match should be the last result row: {lines:?}"
    );
    assert!(
        lines[..match_row].iter().all(|l| l.trim().is_empty()),
        "the empty rows should be above: {lines:?}"
    );
    let query_row = lines
        .iter()
        .position(|line| (line.contains('■') || line.contains('⬝')) && line.contains("[global]"))
        .expect("the query row is drawn");
    assert_eq!(
        query_row - match_row,
        3,
        "there should be a separator and the surface's upper padding: {lines:?}"
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
        scope: Scope::Global,
        total: 3,
        cols: 80,
        rows: 10,
        now: 100,
        // The scanner's frame. Fixed, so a test never depends on when it ran.
        elapsed_ms: 0,
        // Not asking anything: these cover the ordinary search bar.
        confirm: None,
        profile: "default",
    });
    let seen = plain(&rendered);
    // The search bar uses the same glyph, so only the list rows are counted.
    let list: Vec<&str> = seen.lines().take(seen.lines().count() - 3).collect();
    let marked: Vec<&&str> = list.iter().filter(|l| l.contains('❯')).collect();
    assert_eq!(marked.len(), 1, "exactly one marker: {list:?}");
    assert!(
        marked[0].contains("second"),
        "the wrong row is marked: {:?}",
        marked[0]
    );
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
    let seen = plain(&frame_of(&matches, "", 8));
    let lines: Vec<&str> = seen.lines().collect();
    let row_of = |needle: &str| {
        lines
            .iter()
            .position(|l| l.contains(needle))
            .unwrap_or_else(|| panic!("{needle} is not drawn: {lines:?}"))
    };
    // 8 rows: 3 surface, 2 margins, 3 for the list — so the best match is result row 2.
    assert_eq!(
        row_of("best"),
        2,
        "the best match should sit against the surface: {lines:?}"
    );
    assert!(
        row_of("third") < row_of("second") && row_of("second") < row_of("best"),
        "older matches should be further up: {lines:?}"
    );
}

/// Rows alternate: one plain, one on colour 235, so a long list can be read across without the eye
/// losing its place.
#[test]
fn the_rows_alternate() {
    let matches: Vec<_> = (0..4)
        .map(|i| ranked(&format!("row-{i}"), 1, 10 - i, "/home/me", false))
        .collect();
    let rendered = frame_of(&matches, "", 9);
    let row = |needle: &str| {
        rendered
            .lines()
            .find(|l| l.contains(needle))
            .unwrap_or_else(|| panic!("{needle} is not drawn in {rendered:?}"))
    };
    // Index 0 is selected and takes the selection colour; odd rows stripe, even ones do not.
    assert!(
        !row("row-2").contains("48;5;"),
        "an even row should be plain"
    );
    assert!(row("row-1").contains("48;5;235"));
    assert!(row("row-3").contains("48;5;235"));
}

/// Every separator between columns carries the row background too; otherwise the terminal's own
/// background appears as vertical holes through striped and selected rows.
#[test]
fn coloured_rows_have_no_unpainted_column_gaps() {
    let matches = [
        ranked("selected", 1, 10, "/home/me", false),
        ranked("striped", 1, 9, "/home/me", false),
    ];
    let rendered = frame_of(&matches, "", 9);
    for needle in ["selected", "striped"] {
        let row = rendered
            .lines()
            .find(|line| line.contains(needle))
            .expect("row is drawn");
        assert!(
            !row.contains("\x1b[0m \x1b["),
            "an unpainted column gap remains in {needle}: {row:?}"
        );
    }
}

/// The confirmation replaces the search bar with a bordered box, in the same three rows.
///
/// Same height on purpose: the list above must not shift while you decide, or the row you are
/// about to delete moves out from under your eye at the moment you are looking at it.
#[test]
fn the_confirmation_is_a_box_in_the_bars_place() {
    let matches = vec![ranked("cargo build", 3, 1_000, "/here", true)];
    let asking = plain(&frame(&Frame {
        matches: &matches,
        selected: 0,
        offset: 0,
        query: "",
        elapsed_ms: 0,
        confirm: Some(false),
        profile: "default",
        scope: Scope::Global,
        total: 1,
        cols: 40,
        rows: 10,
        now: 1_000_000_000,
    }));
    let lines: Vec<&str> = asking.lines().collect();
    let top = lines
        .iter()
        .position(|l| l.contains('╭'))
        .expect("a top edge");
    assert!(lines[top].contains('╮'), "{:?}", lines[top]);
    assert!(
        lines[top + 1].contains("delete from history?"),
        "{:?}",
        lines[top + 1]
    );
    assert!(lines[top + 1].contains("yes") && lines[top + 1].contains("no"));
    assert!(lines[top + 2].contains('╰') && lines[top + 2].contains('╯'));
    // The sides are drawn, so it reads as a box rather than two stray rules.
    assert!(lines[top + 1].starts_with('│'), "{:?}", lines[top + 1]);
    assert!(
        lines[top + 1].trim_end().ends_with('│'),
        "{:?}",
        lines[top + 1]
    );
    // And the search bar is gone while the question is up.
    assert!(!asking.contains("[global]"), "the bar is still drawn");
}

/// Every row of the box is the same width as the screen, or the border would step in and out.
#[test]
fn the_box_squares_up() {
    let matches = vec![ranked("cargo build", 3, 1_000, "/here", true)];
    for yes in [true, false] {
        let asking = plain(&frame(&Frame {
            matches: &matches,
            selected: 0,
            offset: 0,
            query: "",
            elapsed_ms: 0,
            confirm: Some(yes),
            profile: "default",
            scope: Scope::Global,
            total: 1,
            cols: 44,
            rows: 10,
            now: 1_000_000_000,
        }));
        for line in asking
            .lines()
            .filter(|l| l.contains('│') || l.contains('╭') || l.contains('╰'))
        {
            assert_eq!(
                line.chars().count(),
                44,
                "a box row is not the screen's width: {line:?}"
            );
        }
    }
}

/// The question and its buttons sit in the middle of the box, not against one edge.
#[test]
fn the_question_is_centred() {
    let matches = vec![ranked("cargo build", 3, 1_000, "/here", true)];
    for cols in [40usize, 60, 80] {
        let asking = plain(&frame(&Frame {
            matches: &matches,
            selected: 0,
            offset: 0,
            query: "",
            elapsed_ms: 0,
            confirm: Some(false),
            profile: "default",
            scope: Scope::Global,
            total: 1,
            cols,
            rows: 10,
            now: 1_000_000_000,
        }));
        let row = asking
            .lines()
            .find(|l| l.contains("delete from history?"))
            .expect("the question row");
        let inner: String = row.chars().skip(1).take(cols - 2).collect();
        let left = inner.len() - inner.trim_start().len();
        let right = inner.len() - inner.trim_end().len();
        // Within one cell: an odd leftover cannot be split evenly.
        assert!(
            left.abs_diff(right) <= 1,
            "not centred at {cols} cols: {left} left, {right} right"
        );
        // And the buttons are bracketed.
        assert!(row.contains("[ yes ]"), "{row:?}");
        assert!(row.contains("[ no ]"), "{row:?}");
    }
}

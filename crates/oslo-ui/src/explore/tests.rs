use super::*;

fn flat(text: &str) -> Cell {
    Cell::Flat(text.to_string())
}

fn sheet(columns: &[&str], rows: &[&[&str]]) -> Sheet {
    Sheet {
        title: "explore".to_string(),
        columns: columns.iter().map(|c| c.to_string()).collect(),
        numeric: vec![false; columns.len()],
        rows: rows
            .iter()
            .map(|row| row.iter().map(|c| flat(c)).collect())
            .collect(),
    }
}

/// Escapes stripped, so a test asserts on layout rather than on the theme.
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

/// One line per screen row, with the escapes and the carriage returns taken off.
fn drawn(f: &Frame) -> Vec<String> {
    plain(&frame(f))
        .split("\r\n")
        .map(|line| line.trim_start_matches('\r').trim_end().to_string())
        .collect()
}

fn viewing<'a>(sheet: &'a Sheet, shown: &'a [usize]) -> Frame<'a> {
    Frame {
        sheet,
        shown,
        row: 0,
        column: 0,
        top: 0,
        left: 0,
        query: "",
        trail: &[],
        cols: 80,
        rows: 12,
        elapsed_ms: 0,
    }
}

/// The row the header sits on, and the first row of data under it.
///
/// Counted rather than written down, so a change to the chrome moves the tests with it instead of
/// silently asserting about the wrong line.
const HEADER: usize = SCREEN_MARGIN;
const FIRST: usize = HEADER + HEADER_ROWS;

/// Widths come from every row, not from the visible ones — a column that resized as you scrolled
/// would defeat the point of drawing a table at all.
#[test]
fn a_column_is_as_wide_as_its_widest_value_anywhere() {
    let s = sheet(&["a", "bb"], &[&["x", "y"], &["longer", "y"]]);
    assert_eq!(widths(&s), vec![6, 2]);
}

/// A cell wider than the cap is cut rather than allowed to own the screen.
#[test]
fn one_column_cannot_take_the_whole_screen() {
    let wide = "x".repeat(200);
    let s = sheet(&["a"], &[&[&wide]]);
    assert_eq!(widths(&s), vec![WIDEST]);
}

/// A column wider than the terminal still counts as one, or the frame draws no columns at all and
/// looks like a table with nothing in it.
#[test]
fn at_least_one_column_always_fits() {
    assert_eq!(fitting(&[100], 0, 20), 1);
    assert_eq!(fitting(&[], 0, 80), 1);
}

#[test]
fn columns_fill_the_room_and_stop() {
    // 10 + 2 + 10 + 2 + 10 = 34, and one more would be 46.
    assert_eq!(fitting(&[10, 10, 10, 10], 0, 40), 3);
    assert_eq!(fitting(&[10, 10, 10, 10], 2, 40), 2);
}

/// The chrome is counted, or the last row of data is drawn where the legend goes and the frame
/// scrolls by one every time it is painted.
#[test]
fn the_window_leaves_room_for_the_chrome() {
    // A margin, the header, the gap and three-row surface, and the legend.
    assert_eq!(window(12), 5);
    assert_eq!(window(1), 1, "never zero");
}

/// The margin, the header band, the rows, the finder's surface, the legend — in that order, and
/// exactly as many lines as the terminal has.
#[test]
fn the_frame_is_a_header_the_rows_and_the_finders_bar() {
    let s = sheet(&["name", "size"], &[&["a", "1"], &["b", "22"]]);
    let shown = vec![0, 1];
    let lines = drawn(&viewing(&s, &shown));

    assert_eq!(lines[0], "", "a margin at the top: {lines:?}");
    assert_eq!(lines[HEADER].trim_start(), "name  size", "{lines:?}");
    assert!(lines[FIRST].starts_with(" > a"), "the marker: {lines:?}");
    assert_eq!(lines[FIRST + 1], "   b     22", "{lines:?}");
    assert!(
        lines.iter().any(|l| l.contains(">>")),
        "the finder's prompt: {lines:?}"
    );
    assert!(lines.last().unwrap().contains("quit"), "{lines:?}");
    assert_eq!(lines.len(), 12, "one frame is one screen: {lines:?}");
}

/// The alignment the shell decided is the alignment drawn, so a column does not move when you open
/// the table it was in.
#[test]
fn a_numeric_column_is_drawn_right_aligned() {
    let mut s = sheet(&["n"], &[&["9"], &["2315"]]);
    s.numeric = vec![true];
    let shown = vec![0, 1];
    // The second row, because the first wears the cursor's marker.
    assert_eq!(drawn(&viewing(&s, &shown))[FIRST + 1], "   2315");

    s.numeric = vec![false];
    assert_eq!(drawn(&viewing(&s, &shown))[FIRST + 1], "   2315");
    assert_eq!(drawn(&viewing(&s, &shown))[FIRST].trim_start(), "> 9");
}

/// Only the columns that fit are drawn, starting at `left` — that is what scrolling sideways is.
#[test]
fn scrolling_sideways_changes_which_columns_are_drawn() {
    // Three columns thirty cells wide: two fit on an eighty-cell screen and the third does not.
    let wide = "x".repeat(30);
    let s = sheet(&["aaaa", "bbbb", "cccc"], &[&[&wide, &wide, &wide]]);
    let shown = vec![0];
    let head = |f: &Frame| drawn(f)[HEADER].trim_start().to_string();

    let at_the_left = viewing(&s, &shown);
    assert!(
        head(&at_the_left).starts_with("aaaa"),
        "{:?}",
        head(&at_the_left)
    );
    assert!(
        head(&at_the_left).contains("bbbb"),
        "{:?}",
        head(&at_the_left)
    );
    assert!(
        !head(&at_the_left).contains("cccc"),
        "{:?}",
        head(&at_the_left)
    );
    assert!(
        head(&at_the_left).contains("‹ 1-2/3 ›"),
        "and says which: {:?}",
        head(&at_the_left)
    );

    let scrolled = Frame {
        left: 1,
        column: 1,
        ..viewing(&s, &shown)
    };
    assert!(head(&scrolled).starts_with("bbbb"), "{:?}", head(&scrolled));
    assert!(head(&scrolled).contains("cccc"), "{:?}", head(&scrolled));
    assert!(
        head(&scrolled).contains("‹ 2-3/3 ›"),
        "{:?}",
        head(&scrolled)
    );
}

/// A table that fits says nothing about its columns: `‹ 1-3/3 ›` is noise about a fact the screen
/// is already showing.
#[test]
fn a_table_that_fits_does_not_count_its_columns() {
    let s = sheet(&["a", "b"], &[&["1", "2"]]);
    let shown = vec![0];
    assert!(!drawn(&viewing(&s, &shown))[HEADER].contains('‹'));
}

/// The breadcrumb rides in the badge on the search bar — where the finder puts its scope, and for
/// the same reason: it is the one part of the bar you can change from here.
#[test]
fn descending_shows_the_trail_in_the_badge() {
    let s = sheet(&["value"], &[&["p"]]);
    let shown = vec![0];
    let trail = vec!["explore".to_string(), "meta".to_string()];
    let lines = drawn(&Frame {
        trail: &trail,
        ..viewing(&s, &shown)
    });
    let bar = lines
        .iter()
        .find(|l| l.contains(">>"))
        .expect("a search bar");
    assert!(bar.contains("explore › meta › explore"), "{bar:?}");
    assert!(bar.contains("1/1"), "and where you are: {bar:?}");
}

/// The legend offers `enter` only where there is something to open and `bksp` only where there is
/// somewhere to go back to — a key that does nothing is worse than no key at all.
#[test]
fn the_legend_offers_only_the_keys_that_do_something() {
    let flat = sheet(&["a"], &[&["x"]]);
    let shown = vec![0];
    let lines = drawn(&viewing(&flat, &shown));
    let legend = lines.last().unwrap();
    assert!(!legend.contains("open"), "{legend}");
    assert!(!legend.contains("back"), "{legend}");

    let mut nested = flat.clone();
    nested.rows[0][0] = Cell::Nested {
        summary: "<1 item>".to_string(),
        sheet: Box::new(sheet(&["value"], &[&["p"]])),
    };
    let trail = vec!["explore".to_string()];
    let lines = drawn(&Frame {
        trail: &trail,
        ..viewing(&nested, &shown)
    });
    let legend = lines.last().unwrap();
    assert!(legend.contains("open"), "{legend}");
    assert!(legend.contains("back"), "{legend}");
}

/// A filter that matched nothing says so. `1/0` is what a position counter says about a list with
/// no positions in it, and it reads as a place in the table rather than as the absence of one.
#[test]
fn a_filter_that_matched_nothing_says_so() {
    let s = sheet(&["a"], &[&["x"]]);
    let lines = drawn(&Frame {
        query: "zzz",
        ..viewing(&s, &[])
    });
    let bar = lines
        .iter()
        .find(|l| l.contains(">>"))
        .expect("a search bar");
    assert!(bar.contains("no match"), "{bar:?}");
    assert!(bar.contains("zzz"), "and what was typed: {bar:?}");
}

/// An empty sheet is refused before the terminal is touched: a viewer that took the screen and
/// showed nothing would read as a hang, and it is answered ahead of `NoTerminal` because "there
/// was nothing to look at" is the truer of the two.
#[test]
fn an_empty_sheet_never_opens() {
    assert_eq!(
        crate::explore::open(sheet(&["a"], &[]), crate::matching::Fuzzy::Smart),
        crate::explore::Outcome::Empty
    );
}

/// The list reads top-down. The finder grows its list upward because rank is what a search
/// answers; a table's row order is data, so the first row is the first row.
#[test]
fn the_table_is_not_reversed() {
    assert!(!look().reverse);
    assert_eq!(look().filter_at, crate::ask::look::Where::Bottom);
}

/// A column name is padded to its column's width, so the header line ends in blank. Cutting that
/// blank to make room for the span put an ellipsis in the middle of nothing — the marker must mean
/// a name really did not fit.
#[test]
fn the_column_span_does_not_ellipsise_the_padding() {
    let wide = "x".repeat(30);
    let s = sheet(&["aaaa", "bbbb", "cccc"], &[&[&wide, &wide, &wide]]);
    let shown = vec![0];
    let head = drawn(&viewing(&s, &shown))[HEADER].clone();
    assert!(head.contains("‹ 1-2/3 ›"), "{head:?}");
    assert!(!head.contains('…'), "nothing was cut: {head:?}");
}

/// The selection's background, as SGR parameters, so a test can find it without hard-coding a
/// colour the theme owns. Empty when the terminal is not being coloured at all.
fn selection_bg() -> String {
    let probe = Style {
        bg: look().selected.bg,
        ..Style::default()
    }
    .paint("x", theme::depth());
    probe
        .strip_prefix("\x1b[")
        .and_then(|rest| rest.split('m').next())
        .unwrap_or("")
        .to_string()
}

/// **The cursor is a cell, not a row.** A list has one dimension and the finder can paint the whole
/// of it; here the thing being pointed at is one column of one row, and painting the row would say
/// the row was picked. The marker is what says which row.
#[test]
fn the_selection_is_one_cell_wide() {
    let s = sheet(&["a", "b", "c"], &[&["1", "2", "3"], &["4", "5", "6"]]);
    let shown = vec![0, 1];
    let bg = selection_bg();
    assert!(!bg.is_empty(), "the test needs a coloured theme");

    let painted = frame(&Frame {
        column: 1,
        ..viewing(&s, &shown)
    });
    let row = painted
        .split("\r\n")
        .find(|line| plain(line).contains("> 1"))
        .expect("the row under the cursor");
    assert_eq!(
        row.matches(&bg).count(),
        1,
        "one cell wears the selection, not the row: {row:?}"
    );
    // And it is the cell the cursor is in: the run carrying that background holds `2`, not `1`.
    let after = row
        .split(&format!("{bg}m"))
        .nth(1)
        .expect("the selected run");
    assert!(
        plain(after).starts_with('2'),
        "the cursor's own column: {after:?}"
    );
}

/// The row keeps whatever the list painted it — so the stripe still runs edge to edge under the
/// cursor rather than being interrupted by it.
#[test]
fn the_cursors_row_keeps_its_stripe() {
    let s = sheet(&["a"], &[&["1"], &["2"]]);
    let shown = vec![0, 1];
    let striped = |row: usize| {
        let painted = frame(&Frame {
            row,
            ..viewing(&s, &shown)
        });
        let want = format!("> {}", row + 1);
        painted
            .split("\r\n")
            .find(|line| plain(line).contains(&want))
            .expect("the row under the cursor")
            .to_string()
    };
    // Row 1 is the striped one; row 0 is not. Both carry the cursor in turn, and the stripe is
    // unaffected by which.
    assert!(
        striped(1).len() > striped(0).len(),
        "the odd row is striped"
    );
}

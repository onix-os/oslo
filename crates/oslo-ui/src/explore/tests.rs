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

fn drawn(f: &Frame) -> Vec<String> {
    plain(&frame(f))
        .split("\r\n")
        .map(|line| line.trim_end().to_string())
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
        rows: 10,
    }
}

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
    assert_eq!(window(10, ""), 6);
    assert_eq!(
        window(10, "q"),
        5,
        "the filter takes a row when there is one"
    );
    assert_eq!(window(1, ""), 1, "never zero");
}

/// A heading, the column names, a rule, the rows, and the legend — in that order, and no more lines
/// than the terminal has.
#[test]
fn the_frame_has_a_heading_a_header_a_rule_and_the_rows() {
    let s = sheet(&["name", "size"], &[&["a", "1"], &["b", "22"]]);
    let shown = vec![0, 1];
    let lines = drawn(&viewing(&s, &shown));

    assert!(lines[0].starts_with("explore"), "{lines:?}");
    assert!(lines[0].contains("row 1/2"), "{lines:?}");
    assert_eq!(lines[1], "name  size", "{lines:?}");
    assert!(lines[2].starts_with('─'), "{lines:?}");
    assert!(lines[3].starts_with("a "), "{lines:?}");
    assert!(lines[4].starts_with("b "), "{lines:?}");
    assert!(lines.last().unwrap().contains("quit"), "{lines:?}");
    assert_eq!(lines.len(), 10, "one frame is one screen: {lines:?}");
}

/// The alignment the shell decided is the alignment drawn, so a column does not move when you open
/// the table it was in.
#[test]
fn a_numeric_column_is_drawn_right_aligned() {
    let mut s = sheet(&["n"], &[&["9"], &["2315"]]);
    s.numeric = vec![true];
    let shown = vec![0, 1];
    let lines = drawn(&viewing(&s, &shown));
    assert_eq!(lines[3], "   9", "{lines:?}");
    assert_eq!(lines[4], "2315", "{lines:?}");

    s.numeric = vec![false];
    let lines = drawn(&viewing(&s, &shown));
    assert_eq!(lines[3], "9", "{lines:?}");
}

/// Only the columns that fit are drawn, starting at `left` — that is what scrolling sideways is.
#[test]
fn scrolling_sideways_changes_which_columns_are_drawn() {
    let s = sheet(&["aaaa", "bbbb", "cccc"], &[&["1", "2", "3"]]);
    let shown = vec![0];
    let narrow = Frame {
        cols: 10,
        ..viewing(&s, &shown)
    };
    assert_eq!(drawn(&narrow)[1], "aaaa  bbbb");

    let scrolled = Frame {
        left: 1,
        column: 1,
        cols: 10,
        ..viewing(&s, &shown)
    };
    assert_eq!(drawn(&scrolled)[1], "bbbb  cccc");
}

/// The breadcrumb says where you are, because two levels down a `value` column is otherwise
/// indistinguishable from any other table.
#[test]
fn descending_shows_the_trail() {
    let s = sheet(&["value"], &[&["p"]]);
    let shown = vec![0];
    let trail = vec!["explore".to_string(), "meta".to_string()];
    let lines = drawn(&Frame {
        trail: &trail,
        ..viewing(&s, &shown)
    });
    assert!(
        lines[0].starts_with("explore › meta › explore"),
        "{lines:?}"
    );
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

/// An empty sheet draws a screen that says so rather than one that looks like a hang.
#[test]
fn no_rows_says_no_rows() {
    let s = sheet(&["a"], &[]);
    let lines = drawn(&viewing(&s, &[]));
    assert!(lines[0].contains("no rows"), "{lines:?}");
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

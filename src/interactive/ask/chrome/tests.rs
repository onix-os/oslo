//! What surrounds a widget, asserted on the rows rather than on the escapes.

use super::*;

/// A frame as the widgets build them: rows joined by `\r\n`, each with its own erase.
fn frame(rows: &[&str]) -> String {
    rows.iter()
        .map(|row| format!("\r\x1b[K{row}"))
        .collect::<Vec<_>>()
        .join("\r\n")
}

/// The rows of a wrapped frame, with the redraw escapes taken back off.
fn drawn(chrome: &Chrome, rows: &[&str]) -> Vec<String> {
    super::rows_of(&chrome.wrap(&frame(rows)))
}

/// Default chrome changes nothing at all. Every widget drew this way before the module existed and
/// must still.
#[test]
fn default_chrome_leaves_the_frame_alone() {
    let chrome = Chrome::default();
    assert!(chrome.is_plain());
    assert_eq!(drawn(&chrome, &["one", "two"]), vec!["one", "two"]);
}

/// A border hugs the widest row, and every row is padded to it so the right edge is straight.
#[test]
fn a_content_border_hugs_the_widest_row() {
    let chrome = Chrome {
        border: Border::Square,
        ..Chrome::default()
    };
    let rows = drawn(&chrome, &["short", "much longer row"]);
    assert_eq!(rows.len(), 4, "two rows plus a lid at each end: {rows:?}");
    assert_eq!(rows[0], "┌───────────────┐");
    assert_eq!(rows[1], "│short          │");
    assert_eq!(rows[2], "│much longer row│");
    assert_eq!(rows[3], "└───────────────┘");
}

/// Every row of a bordered frame is the same width — the thing that is wrong the moment a caller
/// measures in bytes instead of cells.
#[test]
fn a_bordered_frame_is_rectangular() {
    let chrome = Chrome {
        border: Border::Rounded,
        ..Chrome::default()
    };
    let rows = drawn(&chrome, &["a", "bbbbb", "cc"]);
    let widths: Vec<usize> = rows.iter().map(|r| printed(r)).collect();
    assert!(
        widths.iter().all(|w| *w == widths[0]),
        "ragged: {widths:?} for {rows:?}"
    );
}

/// Colour inside the frame is not width. A row that is mostly escape sequences must not stretch
/// the box.
#[test]
fn colour_inside_does_not_widen_the_box() {
    let chrome = Chrome {
        border: Border::Square,
        ..Chrome::default()
    };
    let plain = drawn(&chrome, &["abcde"]);
    let painted = drawn(&chrome, &["\x1b[31mabcde\x1b[0m"]);
    assert_eq!(printed(&plain[0]), printed(&painted[0]));
}

/// `Fit::Full` reaches the edges of the terminal rather than the content.
#[test]
fn a_full_border_is_wider_than_the_content() {
    let hug = Chrome {
        border: Border::Square,
        ..Chrome::default()
    };
    let full = Chrome {
        border: Border::Square,
        fit: Fit::Full,
        ..Chrome::default()
    };
    let narrow = printed(&drawn(&hug, &["x"])[0]);
    let wide = printed(&drawn(&full, &["x"])[0]);
    assert!(wide > narrow, "full {wide} should exceed content {narrow}");
    assert_eq!(wide, width::terminal_cols(), "and reach the edge");
}

/// Centring moves the whole frame right by half the slack, and every row by the same amount — a
/// frame indented row-by-row would shear.
#[test]
fn centring_moves_every_row_together() {
    let chrome = Chrome {
        align_x: Place::Center,
        ..Chrome::default()
    };
    let rows = drawn(&chrome, &["aaa", "b"]);
    let lead = |row: &str| row.len() - row.trim_start().len();
    assert_eq!(lead(&rows[0]), lead(&rows[1]), "sheared: {rows:?}");
    assert!(lead(&rows[0]) > 0, "nothing moved: {rows:?}");
}

/// Right-aligned puts the widest row against the right edge.
#[test]
fn end_alignment_reaches_the_right_edge() {
    let chrome = Chrome {
        align_x: Place::End,
        ..Chrome::default()
    };
    let rows = drawn(&chrome, &["abc"]);
    assert_eq!(printed(&rows[0]), width::terminal_cols());
}

/// Vertical placement is blank rows above, and **only** on a screen the widget owns. Inline it
/// would push the caller's transcript up rather than move the widget down.
#[test]
fn vertical_placement_needs_a_screen_of_its_own() {
    let inline = Chrome {
        align_y: Place::Center,
        ..Chrome::default()
    };
    assert_eq!(inline.top_margin(4), 0, "inline must not add rows");

    let owned = Chrome {
        align_y: Place::Center,
        fullscreen: true,
        ..Chrome::default()
    };
    assert!(owned.top_margin(4) > 0, "on its own screen it should");
}

/// A frame taller than the screen is not pushed off the top of it.
#[test]
fn a_tall_frame_is_not_pushed_off_the_screen() {
    let chrome = Chrome {
        align_y: Place::Center,
        fullscreen: true,
        ..Chrome::default()
    };
    assert_eq!(chrome.top_margin(width::terminal_rows() + 50), 0);
}

/// The names a config writes, and a refusal for anything else — a typo must not silently pick a
/// placement the caller did not ask for.
#[test]
fn the_names_are_the_ones_a_config_writes() {
    assert_eq!(Fit::parse("content"), Some(Fit::Content));
    assert_eq!(Fit::parse("full"), Some(Fit::Full));
    assert_eq!(Fit::parse("fully"), None);

    assert_eq!(Place::parse("left"), Some(Place::Start));
    assert_eq!(Place::parse("top"), Some(Place::Start));
    assert_eq!(Place::parse("centre"), Some(Place::Center));
    assert_eq!(Place::parse("bottom"), Some(Place::End));
    assert_eq!(Place::parse("middleish"), None);
}

/// A border and a placement compose: the box is built first, then moved.
#[test]
fn a_border_and_a_placement_compose() {
    let chrome = Chrome {
        border: Border::Square,
        align_x: Place::Center,
        ..Chrome::default()
    };
    let rows = drawn(&chrome, &["hi"]);
    assert_eq!(rows.len(), 3);
    assert!(rows[0].trim_start().starts_with('┌'), "{rows:?}");
    assert!(rows[0].starts_with(' '), "it should have moved: {rows:?}");
}

/// An empty frame does not panic and does not produce a box with no inside.
#[test]
fn an_empty_frame_is_survivable() {
    let chrome = Chrome {
        border: Border::Square,
        ..Chrome::default()
    };
    let rows = drawn(&chrome, &[""]);
    assert_eq!(rows.len(), 3, "{rows:?}");
}

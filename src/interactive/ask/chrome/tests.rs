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
    super::rows_of(&chrome.wrap(&frame(rows), &[]))
}

/// Default chrome changes nothing at all. Every widget drew this way before the module existed and
/// must still.
#[test]
fn default_chrome_leaves_the_frame_alone() {
    let chrome = Chrome::default();
    assert!(chrome.is_plain());
    assert_eq!(drawn(&chrome, &["one", "two"]), vec!["one", "two"]);
}

/// A border hugs the widest row, with a cell of padding each side, and every row is squared off so
/// the right edge is straight.
///
/// The padding is the default and not decoration: text touching the wall of its own box reads as a
/// rendering fault.
#[test]
fn a_content_border_hugs_the_widest_row() {
    let chrome = Chrome {
        border: Border::Square,
        ..Chrome::default()
    };
    let rows = drawn(&chrome, &["short", "much longer row"]);
    assert_eq!(rows.len(), 4, "two rows plus a lid at each end: {rows:?}");
    assert_eq!(rows[0], "┌─────────────────┐");
    assert_eq!(rows[1], "│ short           │");
    assert_eq!(rows[2], "│ much longer row │");
    assert_eq!(rows[3], "└─────────────────┘");
}

/// Padding is a number, not a fact. Zero puts the text back against the wall.
#[test]
fn padding_is_configurable() {
    let none = Chrome {
        border: Border::Square,
        padding_x: 0,
        ..Chrome::default()
    };
    assert_eq!(drawn(&none, &["hi"])[1], "│hi│");

    let wide = Chrome {
        border: Border::Square,
        padding_x: 3,
        ..Chrome::default()
    };
    assert_eq!(drawn(&wide, &["hi"])[1], "│   hi   │");

    let tall = Chrome {
        border: Border::Square,
        padding_y: 1,
        ..Chrome::default()
    };
    let rows = drawn(&tall, &["hi"]);
    assert_eq!(rows.len(), 5, "a blank row above and below: {rows:?}");
    // Trimmed of the walls as well as the spaces — `│` is not whitespace.
    let inside = |row: &str| row.trim_matches(|c| c == '│' || c == ' ').to_string();
    assert_eq!(inside(&rows[1]), "", "{rows:?}");
    assert_eq!(inside(&rows[2]), "hi", "{rows:?}");
    assert_eq!(inside(&rows[3]), "", "{rows:?}");
}

/// **Padding needs a border.** Without one it would be an indent, which `align_x` already is, and a
/// widget that quietly moved two cells right for no visible reason is a bug nobody can name.
#[test]
fn padding_without_a_border_does_nothing() {
    let chrome = Chrome {
        padding_x: 4,
        padding_y: 2,
        ..Chrome::default()
    };
    assert_eq!(drawn(&chrome, &["hi"]), vec!["hi"]);
}

// ------------------------------------------------------------------ the legend

/// The rule spans the box it is in, wall to wall.
///
/// It used to be measured from the content *above* it, before the border was applied — so in a box
/// it came out a fifth of the width and read as damage rather than as a tear-off line.
#[test]
fn the_rule_reaches_both_walls() {
    let chrome = Chrome {
        border: Border::Square,
        ..Chrome::default()
    };
    // Content narrower than the legend, so the box is sized by the keys and the rule has to reach
    // past the content to both walls — the case the old measure-the-content-above rule got wrong.
    let keys = [("↑↓", "move"), ("enter", "confirm"), ("esc", "cancel")];
    let rows = super::rows_of(&chrome.wrap(&frame(&["a", "b"]), &keys));
    let rule = rows
        .iter()
        .find(|r| r.contains('-'))
        .expect("a rule somewhere");
    // Every row is the same width, rule included: it spans wall to wall rather than stopping at
    // whatever the content above it happened to be.
    assert_eq!(printed(rule), printed(&rows[0]), "ragged: {rule:?}");
    let bare = rule.trim_matches(|c| c == '│' || c == ' ');
    assert!(
        printed(bare) > 20,
        "the rule should span the legend, not the content: {bare:?}"
    );
}

/// A blank row between the content and the rule, so the thing you are answering and the note about
/// the widget do not read as one block.
#[test]
fn there_is_a_gap_above_the_legend() {
    let chrome = Chrome::default();
    let rows = super::rows_of(&chrome.wrap(&frame(&["a"]), &[("q", "quit")]));
    assert_eq!(rows.len(), 4, "content, gap, rule, keys: {rows:?}");
    assert_eq!(rows[1].trim(), "", "no gap: {rows:?}");
}

/// The gap is a number too.
#[test]
fn the_gap_is_configurable() {
    let none = Chrome {
        legend_gap: 0,
        ..Chrome::default()
    };
    assert_eq!(
        super::rows_of(&none.wrap(&frame(&["a"]), &[("q", "quit")])).len(),
        3
    );
    let wide = Chrome {
        legend_gap: 3,
        ..Chrome::default()
    };
    assert_eq!(
        super::rows_of(&wide.wrap(&frame(&["a"]), &[("q", "quit")])).len(),
        6
    );
}

/// With the legend off there is no gap and no rule either — the rows go back to the content.
#[test]
fn no_legend_means_no_gap_and_no_rule() {
    let chrome = Chrome {
        legend: false,
        ..Chrome::default()
    };
    let rows = super::rows_of(&chrome.wrap(&frame(&["a"]), &[("q", "quit")]));
    assert_eq!(rows, vec!["a"]);
    assert_eq!(chrome.legend_rows(), 0);
}

/// What a widget reserves matches what is drawn. A window sized against a different number is how
/// a widget eats rows of the caller's transcript, one per keystroke.
#[test]
fn the_reserved_rows_match_the_drawn_ones() {
    for chrome in [
        Chrome::default(),
        Chrome {
            border: Border::Square,
            ..Chrome::default()
        },
        Chrome {
            border: Border::Square,
            padding_y: 2,
            legend_gap: 0,
            ..Chrome::default()
        },
        Chrome {
            legend: false,
            ..Chrome::default()
        },
    ] {
        let content = ["a", "b", "c"];
        let rows = super::rows_of(&chrome.wrap(&frame(&content), &[("q", "quit")]));
        assert_eq!(
            rows.len(),
            content.len() + chrome.extra_rows(),
            "reserved {} but drew {} for {chrome:?}",
            chrome.extra_rows(),
            rows.len()
        );
    }
}

/// **The leading row a frame carries is the caller's, not content.** Every widget's frame starts
/// with `\r\n` because it draws on the row below the prompt; counting it as a row put a blank line
/// inside the top of every box.
#[test]
fn the_leading_row_is_not_content() {
    let chrome = Chrome {
        border: Border::Square,
        ..Chrome::default()
    };
    let with_lead = chrome.wrap(&format!("\r\n{}", frame(&["hi"])), &[]);
    assert!(with_lead.starts_with("\r\n"), "the lead is kept");
    let rows = super::rows_of(&with_lead);
    // "", top, content, bottom — and no blank row between the top and the content.
    assert_eq!(rows.len(), 4, "{rows:?}");
    assert!(rows[2].contains("hi"), "a blank row crept in: {rows:?}");
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

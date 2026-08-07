//! How a list is drawn, asserted on the rows rather than on the escapes.

use super::*;

/// The frame's rows, with the redraw escapes and the colour taken back off.
fn drawn(look: &Look, rows: &[&str], view: &View<'_>) -> Vec<String> {
    let rows: Vec<Row> = rows.iter().map(|r| Row::new(*r)).collect();
    look.frame(&rows, view)
        .split("\r\n")
        .skip(1)
        .map(|row| {
            let row = row.trim_start_matches('\r');
            plain(row.strip_prefix("\x1b[K").unwrap_or(row))
        })
        .collect()
}

/// Escapes stripped, so a test can assert on what is on screen.
fn plain(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars().peekable();
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

fn view(height: usize, total: usize) -> View<'static> {
    View {
        selected: 0,
        offset: 0,
        height,
        query: "",
        matched: total,
        total,
        marked: 0,
        cols: 40,
        filtering: false,
    }
}

/// The default look draws what the widgets drew before this module existed: a marker on the
/// selected row, two reserved cells on the others, and nothing else.
#[test]
fn the_default_look_is_the_old_one() {
    let look = Look::default();
    assert!(look.is_plain());
    let rows = drawn(&look, &["alpha", "beta"], &view(2, 2));
    assert_eq!(rows, vec!["❯ alpha", "  beta"]);
}

/// A filter at the bottom is drawn after the list, and one at the top before it. This is the
/// single field that turns a menu into a finder.
#[test]
fn the_filter_sits_where_it_was_put() {
    let rows = ["alpha", "beta"];
    let mut view = view(2, 2);
    view.filtering = true;

    let top = drawn(&Look::default(), &rows, &view);
    assert!(top[0].contains("type to filter"), "{top:?}");

    let bottom = Look {
        filter_at: Where::Bottom,
        ..Look::default()
    };
    let rows = drawn(&bottom, &rows, &view);
    assert!(rows[2].contains("type to filter"), "{rows:?}");
    assert!(rows[0].contains("alpha"), "{rows:?}");
}

/// Reversed, the best match is the row nearest the filter — which is the point of putting the
/// filter at the bottom in the first place.
#[test]
fn reversed_puts_the_best_match_nearest_the_filter() {
    let look = Look {
        filter_at: Where::Bottom,
        reverse: true,
        ..Look::default()
    };
    let mut v = view(3, 3);
    v.filtering = true;
    let rows = drawn(&look, &["best", "next", "last"], &v);
    assert!(rows[2].contains("best"), "nearest the bar: {rows:?}");
    assert!(rows[0].contains("last"), "furthest: {rows:?}");
}

/// Unused rows go at the top when the list grows upward, so the block stays against the filter
/// rather than floating.
#[test]
fn an_unfilled_reversed_list_leaves_its_gap_at_the_top() {
    let look = Look {
        filter_at: Where::Bottom,
        reverse: true,
        width: Width::Full,
        ..Look::default()
    };
    let mut v = view(4, 1);
    v.filtering = true;
    let rows = drawn(&look, &["only"], &v);
    assert!(rows[3].contains("only"), "{rows:?}");
    assert!(rows[0].trim().is_empty(), "{rows:?}");
}

/// A full-width row is padded to the room it was given, so a background reaches both edges. A
/// stripe that stopped at the last letter would be a highlighted word, not a ruler.
#[test]
fn a_full_width_row_reaches_the_edge() {
    let look = Look {
        width: Width::Full,
        ..Look::default()
    };
    let rows = drawn(&look, &["short"], &view(1, 1));
    assert_eq!(
        crate::interactive::prompt::printed_width(&rows[0]),
        40,
        "{rows:?}"
    );
}

/// A content-width row stops at its text, which is what an inline widget in a border wants.
#[test]
fn a_content_width_row_stops_at_its_text() {
    let rows = drawn(&Look::default(), &["short"], &view(1, 1));
    assert_eq!(rows[0], "❯ short");
}

/// The stripe follows the row's place in the list, not its place on screen — otherwise the bands
/// crawl as the list scrolls under them.
#[test]
fn the_stripe_does_not_crawl_when_the_list_scrolls() {
    let look = Look {
        width: Width::Full,
        stripe: Some(Style {
            bg: Some(Color::Indexed(235)),
            ..Style::default()
        }),
        ..Look::default()
    };
    let rows: Vec<Row> = ["a", "b", "c", "d"].iter().map(|r| Row::new(*r)).collect();
    let striped = |offset: usize| {
        let v = View {
            selected: 99,
            offset,
            height: 2,
            ..view(2, 4)
        };
        look.frame(&rows, &v)
            .split("\r\n")
            .skip(1)
            .map(|row| row.contains("235"))
            .collect::<Vec<_>>()
    };
    // Row 1 is striped and row 2 is not, whichever of them happens to be on top.
    assert_eq!(striped(0), vec![false, true], "rows 0 and 1");
    assert_eq!(striped(1), vec![true, false], "rows 1 and 2");
}

/// The slots say what the list knows about itself. Without them every widget that wanted a
/// counter had to grow its own flag.
#[test]
fn the_slots_are_filled_from_the_list() {
    let look = Look {
        filter_at: Where::Bottom,
        right: "{n}/{total}".to_string(),
        left: "[{index}] ".to_string(),
        ..Look::default()
    };
    let v = View {
        selected: 2,
        matched: 3,
        filtering: true,
        ..view(3, 57)
    };
    let rows = drawn(&look, &["a", "b", "c"], &v);
    let bar = rows.last().expect("a filter row");
    assert!(bar.contains("3/57"), "{bar:?}");
    assert!(bar.contains("[3]"), "one-based: {bar:?}");
}

/// A template with nothing in it is left exactly alone, braces and all — a slot is a place to put
/// text, and text is allowed to contain a brace.
#[test]
fn a_slot_without_fields_is_left_alone() {
    let v = view(1, 1);
    assert_eq!(Look::slot("plain text", &v), "plain text");
    assert_eq!(Look::slot("", &v), "");
    assert_eq!(Look::slot("{unknown}", &v), "{unknown}");
}

/// Three rows of surface is a panel with the query in the middle. One row is a line. Either way
/// the widget has to reserve exactly what gets drawn.
#[test]
fn the_surface_reserves_the_rows_it_draws() {
    for surface_rows in [1usize, 3, 5] {
        let look = Look {
            filter_at: Where::Bottom,
            surface_rows,
            gap: 1,
            ..Look::default()
        };
        let mut v = view(2, 2);
        v.filtering = true;
        let rows = drawn(&look, &["a", "b"], &v);
        assert_eq!(rows.len(), 2 + 1 + surface_rows, "{surface_rows}: {rows:?}");
        assert_eq!(look.extra_rows(true), surface_rows + 1);
        assert_eq!(look.extra_rows(false), 0, "no filter, no rows");
    }
}

/// The query goes in the middle of the surface, which is what makes three rows read as a panel
/// with something in it rather than a line with two spare rows.
#[test]
fn the_query_is_in_the_middle_of_its_surface() {
    let look = Look {
        surface_rows: 3,
        ..Look::default()
    };
    let mut v = view(1, 1);
    v.filtering = true;
    let rows = drawn(&look, &["a"], &v);
    assert!(rows[0].trim().is_empty(), "{rows:?}");
    assert!(rows[1].contains("type to filter"), "{rows:?}");
    assert!(rows[2].trim().is_empty(), "{rows:?}");
}

/// A checked box is the lead column and the cursor is the marker: two columns, because they
/// answer different questions. Folding them together left `--multi` with no cursor at all.
#[test]
fn the_cursor_and_the_checkbox_are_two_columns() {
    let rows = vec![
        Row {
            text: "alpha".to_string(),
            lead: "◉ ".to_string(),
            marked: true,
            trail: String::new(),
            tint: None,
        },
        Row {
            text: "beta".to_string(),
            lead: "◯ ".to_string(),
            marked: false,
            trail: String::new(),
            tint: None,
        },
    ];
    let drawn: Vec<String> = Look::default()
        .frame(&rows, &view(2, 2))
        .split("\r\n")
        .skip(1)
        .map(plain)
        .collect();
    assert!(drawn[0].contains("❯ ◉ alpha"), "{drawn:?}");
    assert!(drawn[1].contains("  ◯ beta"), "{drawn:?}");
}

/// A trailing column is drawn hard right and takes room from the text, so a long label cannot
/// push it off the row.
#[test]
fn a_trail_keeps_its_room() {
    let look = Look {
        width: Width::Full,
        ..Look::default()
    };
    let rows = vec![Row {
        text: "a very long command line that will not fit beside it".to_string(),
        lead: String::new(),
        marked: false,
        trail: " 118×".to_string(),
        tint: None,
    }];
    let drawn: Vec<String> = look
        .frame(&rows, &view(1, 1))
        .split("\r\n")
        .skip(1)
        .map(plain)
        .collect();
    assert!(drawn[0].ends_with(" 118×"), "{drawn:?}");
    assert_eq!(crate::interactive::prompt::printed_width(&drawn[0]), 40);
}

/// The preset is the combination, not the sugar: a bottom filter without `reverse` puts the best
/// match furthest from the cursor, and a stripe without full-width rows paints a word.
#[test]
fn the_history_preset_is_a_working_combination() {
    let look = Preset::History.look();
    assert_eq!(look.filter_at, Where::Bottom);
    assert!(look.reverse, "or the best match is furthest from the bar");
    assert_eq!(look.width, Width::Full, "or the stripe is a coloured word");
    assert!(look.stripe.is_some());
    assert!(!look.is_plain());
    assert!(Preset::Plain.look().is_plain());
}

/// Every name a config can write is one the parser knows, in both directions.
#[test]
fn the_names_are_the_ones_a_script_writes() {
    assert_eq!(Where::parse("bottom"), Some(Where::Bottom));
    assert_eq!(Where::parse("TOP"), Some(Where::Top));
    assert_eq!(Where::parse("sideways"), None);
    assert_eq!(Width::parse("full"), Some(Width::Full));
    assert_eq!(Width::parse("content"), Some(Width::Content));
    assert_eq!(Preset::parse("history"), Some(Preset::History));
    assert_eq!(Preset::parse("menu"), Some(Preset::Menu));
    assert_eq!(Preset::parse("nonsense"), None);
}

/// A list taller than it has rows for does not panic, and neither does an empty one.
#[test]
fn an_empty_list_is_survivable() {
    let rows = drawn(&Look::default(), &[], &view(3, 0));
    assert_eq!(rows.len(), 3, "{rows:?}");
    let narrow = View {
        cols: 1,
        ..view(1, 1)
    };
    assert_eq!(drawn(&Look::default(), &["wide"], &narrow).len(), 1);
}

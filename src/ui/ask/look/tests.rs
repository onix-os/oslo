//! How a list is drawn, asserted on the rows rather than on the escapes.

use super::*;

/// The frame's rows, with the redraw escapes and the colour taken back off.
fn drawn(look: &Look, rows: &[&str], view: &View<'_>) -> Vec<String> {
    let rows: Vec<Row> = rows.iter().map(|r| Row::new(*r)).collect();
    rendered(look, &rows, view)
}

/// The frame's rows for a list of already-built [`Row`]s, escapes and redraws stripped.
fn rendered(look: &Look, rows: &[Row], view: &View<'_>) -> Vec<String> {
    look.frame(rows, view)
        .split("\r\n")
        .skip(1)
        .map(|row| {
            let row = row.trim_start_matches('\r');
            plain(row.strip_prefix("\x1b[K").unwrap_or(row))
        })
        .collect()
}

/// The row the query is on, which is not the last row of a three-row surface.
fn bar_of(rows: &[String]) -> String {
    rows.iter()
        .rev()
        .find(|row| !row.trim().is_empty())
        .cloned()
        .unwrap_or_default()
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
        elapsed_ms: 0,
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
    assert_eq!(crate::ui::prompt::printed_width(&rows[0]), 40, "{rows:?}");
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
    assert_eq!(Look::fill("plain text", &v), "plain text");
    assert_eq!(Look::fill("", &v), "");
    assert_eq!(Look::fill("{unknown}", &v), "{unknown}");
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
            meta: Vec::new(),
            tint: None,
        },
        Row {
            text: "beta".to_string(),
            lead: "◯ ".to_string(),
            marked: false,
            trail: String::new(),
            meta: Vec::new(),
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
        meta: Vec::new(),
        tint: None,
    }];
    let drawn: Vec<String> = look
        .frame(&rows, &view(1, 1))
        .split("\r\n")
        .skip(1)
        .map(plain)
        .collect();
    assert!(drawn[0].ends_with(" 118×"), "{drawn:?}");
    assert_eq!(crate::ui::prompt::printed_width(&drawn[0]), 40);
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

/// **The arrows follow the screen, not the list.** A reversed list draws index 0 at the bottom, so
/// Up has to walk towards the far end of it. Without this the highlight moved down when you
/// pressed Up — which is not a preference to argue about.
#[test]
fn the_arrows_follow_the_screen() {
    let plain = Look::default();
    assert_eq!(plain.step(Key::Up), Some(Step::Back));
    assert_eq!(plain.step(Key::Down), Some(Step::On));
    assert_eq!(plain.step(Key::Home), Some(Step::First));
    assert_eq!(plain.step(Key::End), Some(Step::Last));

    let up = Look {
        reverse: true,
        ..Look::default()
    };
    assert_eq!(up.step(Key::Up), Some(Step::On), "Up must go up the screen");
    assert_eq!(up.step(Key::Down), Some(Step::Back));
    assert_eq!(up.step(Key::Home), Some(Step::Last));
    assert_eq!(up.step(Key::End), Some(Step::First));
}

/// Keys that are not movement stay the widget's own business.
#[test]
fn a_key_that_does_not_move_is_not_claimed() {
    let look = Look::default();
    for key in [Key::Accept, Key::Cancel, Key::Char('a'), Key::Left] {
        assert_eq!(look.step(key), None, "{key:?}");
    }
}

/// A step lands inside the list and cannot walk off either end.
#[test]
fn a_step_stays_inside_the_list() {
    assert_eq!(Step::Back.from(0, 5), 0, "no wrap at the top");
    assert_eq!(Step::On.from(4, 5), 4, "no wrap at the bottom");
    assert_eq!(Step::On.from(0, 5), 1);
    assert_eq!(Step::Back.from(3, 5), 2);
    assert_eq!(Step::Last.from(0, 5), 4);
    assert_eq!(Step::First.from(4, 5), 0);
    // An empty list is a real state: the filter matched nothing and a key still arrives.
    assert_eq!(Step::Last.from(0, 0), 0);
    assert_eq!(Step::On.from(0, 0), 0);
}

/// The metadata columns are right-aligned and sized across the whole list, so they form columns
/// down the screen rather than shifting per row.
#[test]
fn the_meta_columns_line_up() {
    let look = Look {
        width: Width::Full,
        ..Look::default()
    };
    let rows = vec![
        Row {
            meta: vec!["1d".to_string(), "118×".to_string()],
            ..Row::new("cargo test")
        },
        Row {
            meta: vec!["5h".to_string(), "3×".to_string()],
            ..Row::new("git status")
        },
    ];
    let drawn = rendered(&look, &rows, &view(2, 2));
    assert!(drawn[0].starts_with("❯ 1d 118× cargo test"), "{drawn:?}");
    // Right-aligned in the same width, so `3×` is pushed over to sit under `118×`.
    assert!(drawn[1].starts_with("  5h   3× git status"), "{drawn:?}");
}

/// Sized across the whole list rather than the visible window: a column that resized as the list
/// scrolled would undo the alignment it exists for.
#[test]
fn the_meta_widths_do_not_change_as_the_list_scrolls() {
    let look = Look {
        width: Width::Full,
        ..Look::default()
    };
    let rows: Vec<Row> = [("1d", "1×"), ("2d", "999999×"), ("3d", "7×")]
        .iter()
        .map(|(when, runs)| Row {
            meta: vec![when.to_string(), runs.to_string()],
            ..Row::new("a command")
        })
        .collect();
    let at = |offset: usize| {
        let v = View {
            offset,
            height: 1,
            ..view(1, 3)
        };
        rendered(&look, &rows, &v)
            .first()
            .cloned()
            .unwrap_or_default()
    };
    // The first row is drawn in a column wide enough for `999999×`, which is not on screen.
    let first = at(0);
    let last = at(2);
    // In cells, not bytes: the marker is a multibyte character and `find` counts bytes, which
    // would make the two rows look misaligned when they are not.
    let column_of =
        |row: &str| crate::ui::prompt::printed_width(row.split("a command").next().unwrap_or(""));
    assert_eq!(column_of(&first), column_of(&last), "{first:?} {last:?}");
}

/// The badge is the one part of the bar with a background, because it is the only part that is a
/// state you can change rather than a fact about what you are looking at.
#[test]
fn the_badge_is_painted_where_the_slot_puts_it() {
    let look = Look {
        filter_at: Where::Bottom,
        right: "{badge} || {n}/{total}".to_string(),
        badge: "[global]".to_string(),
        ..Look::default()
    };
    let mut v = view(1, 9);
    v.filtering = true;
    v.matched = 4;
    let painted = look.frame(&[Row::new("a")], &v);
    // Its own colour, and the counter beside it in the muted one.
    assert!(painted.contains("[global]"), "{painted:?}");
    assert!(
        painted.contains("48;5;1m[global]"),
        "no badge bg: {painted:?}"
    );
    let rows = drawn(&look, &["a"], &v);
    assert!(rows.last().is_some_and(|r| r.contains("4/9")), "{rows:?}");
}

/// A badge nobody asked for leaves no hole where `{badge}` was.
#[test]
fn an_unset_badge_leaves_no_marker_behind() {
    let look = Look {
        filter_at: Where::Bottom,
        right: "{badge} {n}".to_string(),
        ..Look::default()
    };
    let mut v = view(1, 3);
    v.filtering = true;
    let rows = drawn(&look, &["a"], &v);
    let bar = rows.last().expect("a filter row");
    assert!(!bar.contains("{badge}"), "{bar:?}");
    assert!(bar.contains('3'), "the rest of the slot survives: {bar:?}");
}

/// The scanner is what says the widget is live. It costs a redraw per frame, so a look without one
/// must go back to blocking — an animation that wakes an idle prompt is worth having only while
/// something is animating.
#[test]
fn only_a_scanner_asks_for_a_tick() {
    assert_eq!(Look::default().tick_ms(), None);
    let ticking = Preset::History.look();
    assert!(ticking.scanner.is_some());
    assert!(ticking.tick_ms().is_some_and(|ms| ms > 0), "a real delay");
    assert_eq!(Preset::Menu.look().tick_ms(), None);
}

/// The sweep moves with the clock, and the row it is on stays the width it was — a frame that
/// changed width as it animated would shake the whole bar.
#[test]
fn the_sweep_moves_without_moving_anything_else() {
    let look = Preset::History.look();
    let bar_at = |elapsed_ms: u64| {
        let v = View {
            filtering: true,
            elapsed_ms,
            ..view(1, 1)
        };
        bar_of(&drawn(&look, &["a"], &v))
    };
    let (first, later) = (bar_at(0), bar_at(400));
    assert_ne!(first, later, "the sweep should have moved");
    assert_eq!(
        crate::ui::prompt::printed_width(&first),
        crate::ui::prompt::printed_width(&later),
        "{first:?} {later:?}"
    );
}

/// The history preset is the finder's shape, not an approximation of it: sweep, badge slot,
/// counter, stripes, and rows that grow towards the bar.
#[test]
fn the_history_preset_has_the_whole_bar() {
    let look = Preset::History.look();
    assert!(look.scanner.is_some(), "the sweep says it is live");
    assert!(look.right.contains("{badge}"), "somewhere for the scope");
    assert!(look.right.contains("{n}/{total}"), "the counter");
    assert_eq!(look.prompt.trim(), "❯❯", "where typing starts");
    assert_eq!(look.surface_rows, 3, "a panel, not a line");
}

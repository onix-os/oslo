use super::drawn::{cell, clamp};
use super::*;
use oslo_ui::dropdown::display_width;

fn row(pairs: &[(&str, Val)]) -> Record {
    Record::from_pairs(pairs.iter().map(|(n, v)| (*n, v.clone())))
}

/// **The drawn table reads process-wide settings, and one test writes them.**
///
/// Tests run in parallel, so without this the settings test's `max_column = 8` was visible to
/// every other test that draws a table — for as long as it held them. The same guard `plan.rs`
/// puts around its edge counter, for the same reason: shared mutable state needs the tests that
/// touch it to take turns.
static DRAWN: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn drawing() -> std::sync::MutexGuard<'static, ()> {
    DRAWN.lock().unwrap_or_else(|e| e.into_inner())
}

/// Columns keep the order they were set in, and setting one again does not move it.
#[test]
fn a_record_is_ordered() {
    let mut r = Record::new();
    r.set("filesystem", Val::Str("/dev/sda1".into()));
    r.set("size", Val::Size(1024));
    r.set("used", Val::Size(512));
    assert_eq!(r.columns(), ["filesystem", "size", "used"]);
    r.set("filesystem", Val::Str("/dev/sdb1".into()));
    assert_eq!(r.columns(), ["filesystem", "size", "used"], "no reordering");
    assert_eq!(r.get("filesystem"), Some(&Val::Str("/dev/sdb1".into())));
}

/// Rows may disagree about their columns, so the header is the union of them.
#[test]
fn a_tables_columns_are_the_union_of_its_rows() {
    let table = Val::table(vec![
        row(&[("pid", Val::Int(1)), ("cmd", Val::Str("init".into()))]),
        row(&[("pid", Val::Int(2)), ("user", Val::Str("bo".into()))]),
    ]);
    assert_eq!(table.columns(), ["pid", "cmd", "user"]);
    assert!(table.is_table());
}

/// **The invariant the design rests on**: transport is plain and complete, display is for a
/// person. A size reaches a program as a number it can do arithmetic on.
#[test]
fn the_two_renderings_are_different_functions() {
    let _turn = drawing();
    let value = Val::Size(4_509_715_660);
    assert_eq!(render_display(&value), "4.2G");
    assert_eq!(render_transport(&value), "4509715660");

    let table = Val::table(vec![row(&[
        ("name", Val::Str("root".into())),
        ("free", Val::Size(2048)),
    ])]);
    let display = render_display(&table);
    assert!(display.contains("name"), "display has a header: {display}");
    assert!(display.contains("2.0K"), "display humanises: {display}");

    let transport = render_transport(&table);
    assert_eq!(transport, "root\t2048", "transport is plain and complete");
    assert!(!transport.contains("2.0K"));
}

/// An error is one cell's problem. The row it sits in still arrives, and so do the others.
#[test]
fn an_error_is_a_value_and_does_not_stop_the_stream() {
    let _turn = drawing();
    let table = Val::table(vec![
        row(&[("mount", Val::Str("/".into())), ("free", Val::Size(100))]),
        row(&[
            ("mount", Val::Str("/nfs".into())),
            ("free", Val::Error("stale handle".into())),
        ]),
        row(&[("mount", Val::Str("/tmp".into())), ("free", Val::Size(200))]),
    ]);
    let Val::List(rows) = &table else {
        panic!("a table is a list");
    };
    assert_eq!(rows.len(), 3, "every row survives");
    assert!(render_display(&table).contains("stale handle"));
}

/// Durations read the way every other tool writes them. Sizes are the dropdown's now, and
/// tested where they live.
#[test]
fn durations_are_readable() {
    assert_eq!(human_duration(340_000_000), "340ms");
    assert_eq!(human_duration(1_500_000_000), "1.5s");
    assert_eq!(human_duration(150_000_000_000), "2m30s");
}

/// **The separators are in band, so a cell that holds one is spelled rather than written.**
///
/// A tab in a cell used to end that cell and shift every column after it; a newline ended the
/// record and made one row arrive as two. Both silently, on exactly the data a shell meets —
/// a filename with a tab, a `cmdline` spanning lines.
#[test]
fn a_cell_holding_a_separator_does_not_break_the_framing() {
    let table = Val::table(vec![row(&[
        ("name", Val::Str("two\twords".into())),
        ("note", Val::Str("first\nsecond".into())),
    ])]);
    let transport = render_transport(&table);
    assert_eq!(
        transport.lines().count(),
        1,
        "one record is one line: {transport:?}"
    );
    assert_eq!(transport, "two\\twords\tfirst\\nsecond");
    assert_eq!(
        transport.matches('\t').count(),
        1,
        "exactly one cell separator"
    );
}

/// A backslash goes first, or reading back could not tell `\t` the two characters from a tab.
#[test]
fn escaping_round_trips() {
    for original in [
        "plain",
        "a\tb",
        "a\nb",
        "back\\slash",
        "already\\tspelled",
        "\r\n",
        "",
    ] {
        let table = Val::table(vec![row(&[("c", Val::Str(original.into()))])]);
        assert_eq!(
            unescape_cell(&render_transport(&table)),
            original,
            "{original:?} did not survive"
        );
    }
}

/// A nested cell renders with its own separators, and those are caught too — the escape is
/// applied to the rendered cell, not to the string inside it.
#[test]
fn a_nested_cell_cannot_break_the_framing() {
    let table = Val::table(vec![row(&[
        ("id", Val::Int(1)),
        (
            "tags",
            Val::List(vec![Val::Str("a".into()), Val::Str("b".into())]),
        ),
    ])]);
    let transport = render_transport(&table);
    assert_eq!(transport.lines().count(), 1, "got {transport:?}");
    assert_eq!(transport, "1\ta\\nb");
}

/// A column is terminal cells, not characters: a CJK ideograph is two cells wide and a table
/// that counted it as one drew its columns out of line.
#[test]
fn a_wide_column_lines_up() {
    let _turn = drawing();
    let table = Val::table(vec![
        row(&[("name", Val::Str("名前".into())), ("n", Val::Int(1))]),
        row(&[("name", Val::Str("ab".into())), ("n", Val::Int(2))]),
    ]);
    let drawn = render_display(&table);
    // **Cells, not bytes.** `名前` is six bytes and four columns, so a byte offset would call
    // these rows misaligned when they are drawn correctly — the same mistake `pad` was making.
    let starts: Vec<usize> = drawn
        .lines()
        .filter_map(|line| line.find(['1', '2']).map(|at| display_width(&line[..at])))
        .collect();
    assert_eq!(starts.len(), 2, "two data rows: {drawn}");
    assert_eq!(
        starts[0], starts[1],
        "the second column starts in the same cell on both rows:\n{drawn}"
    );
}

/// **A time has a human face too.** It rendered as its raw nanosecond count in *both* faces, so
/// the tagged kind that makes `where 'modified > 2days'` arithmetic also made the column
/// unreadable — the exact trade `Val::Size` exists to avoid.
#[test]
fn a_time_reads_as_a_date_and_transports_as_a_number() {
    let _turn = drawing();
    // 2019-03-05, comfortably outside the six-month window, so the year is shown.
    let old = Val::Time(1_551_744_000_000_000_000);
    let drawn = render_display(&old);
    assert!(
        drawn.contains("2019"),
        "an old time shows the year: {drawn}"
    );
    assert!(!drawn.contains("1551744"), "not the raw count: {drawn}");
    assert_eq!(
        render_transport(&old),
        "1551744000000000000",
        "a program gets the number to compute with"
    );

    // Recent shows the hour instead, which is what distinguishes two files from today.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let recent = render_display(&Val::Time(now * 1_000_000_000));
    assert!(
        recent.contains(':'),
        "a recent time shows the hour: {recent}"
    );
}

/// **A row is one line.** Without a clamp a wide table wraps every row across two or three
/// terminal lines and the columns stop lining up at all. The marker matters as much as the cut:
/// a silently truncated table looks like data that ends there.
#[test]
fn a_wide_line_is_cut_with_a_marker() {
    let _turn = drawing();
    assert_eq!(clamp("short", 20), "short");
    assert_eq!(clamp("exactly-ten", 11), "exactly-ten", "fits exactly");
    let cut = clamp("a rather long line indeed", 10);
    assert_eq!(display_width(&cut), 10, "cut to the room there is");
    assert!(cut.ends_with('…'), "and says it was cut: {cut:?}");
    // A width of nothing is a terminal that would not say, so nothing is cut.
    assert_eq!(clamp("untouched", 0), "untouched");
}

/// **`oslo.table` is the drawn face, and only the drawn face.**
///
/// The two renderers are two functions so that a preference cannot reach another program's
/// standard input. This is the test that says so: every setting here changes the table and none
/// of them changes the transport.
#[test]
fn the_drawn_table_is_configurable_and_the_transport_is_not() {
    let _turn = drawing();
    // The settings are process-wide, so this test owns them for its duration and puts them back.
    let restore = oslo_ui::settings::current().as_ref().clone();
    let table = Val::table(vec![
        row(&[
            ("n", Val::Int(1)),
            ("long", Val::Str("abcdefghijklmnop".into())),
        ]),
        row(&[("n", Val::Int(2)), ("missing", Val::Null)]),
    ]);
    let before = render_transport(&table);

    let mut settings = restore.clone();
    settings.table.index = true;
    settings.table.null = "-".to_string();
    settings.table.max_column = 8;
    oslo_ui::settings::install(settings);

    let drawn = render_display(&table);
    assert!(drawn.starts_with('#'), "an index column leads: {drawn}");
    assert!(
        drawn.contains('-'),
        "a null shows as the null text: {drawn}"
    );
    assert!(
        drawn.contains('…') && !drawn.contains("abcdefghijklmnop"),
        "a wide cell is cut at max_column: {drawn}"
    );

    assert_eq!(
        render_transport(&table),
        before,
        "not one of those settings may reach the transport"
    );
    assert!(render_transport(&table).contains("abcdefghijklmnop"));

    oslo_ui::settings::install(restore);
}

/// `max_column = 0` is how "no limit" is spelled, and the default leaves ordinary cells alone.
#[test]
fn a_cell_is_only_cut_when_it_is_too_wide() {
    let _turn = drawing();
    assert_eq!(cell("short", 60), "short");
    assert_eq!(
        cell("untouched by a limit of none", 0),
        "untouched by a limit of none"
    );
    let cut = cell("abcdefghijklmnop", 8);
    assert_eq!(display_width(&cut), 8, "cut to the room there is: {cut:?}");
    assert!(cut.ends_with('…'));
    assert_eq!(cut.matches('…').count(), 1, "exactly one ellipsis: {cut:?}");
}

/// Transport is never truncated: the program on the other end asked for all of it.
#[test]
fn transport_is_never_clamped() {
    let wide = Val::table(vec![row(&[(
        "c",
        Val::Str("a very long value that no terminal is this wide for, by some way".into()),
    )])]);
    assert!(render_transport(&wide).contains("by some way"));
    assert!(!render_transport(&wide).contains('…'));
}

/// Bytes are never rendered as text: a JPEG through `render_display` is a description, not
/// mojibake.
#[test]
fn binary_is_described_rather_than_mangled() {
    let value = Val::Bytes(vec![0xff, 0xd8, 0xff, 0xe0]);
    assert_eq!(render_display(&value), "<4 bytes>");
}

/// A column of numbers reads down its last digit, and one that is not stays left.
///
/// The mixed column is the point: `9` and `2315` compared by eye need their units in the same
/// place, but a column that switched alignment half way down — because one row held a word — would
/// be harder to read than either choice made consistently.
#[test]
fn numbers_align_right_and_text_aligns_left() {
    let _turn = drawing();
    let restore = oslo_ui::settings::current().as_ref().clone();
    oslo_ui::settings::install(restore.clone());

    let table = Val::table(vec![
        row(&[
            ("name", Val::Str("a".into())),
            ("count", Val::Int(9)),
            ("mixed", Val::Int(9)),
        ]),
        row(&[
            ("name", Val::Str("bbbb".into())),
            ("count", Val::Int(2315)),
            ("mixed", Val::Str("word".into())),
        ]),
    ]);
    let drawn = render_display(&table);
    let lines: Vec<&str> = drawn.lines().collect();
    assert!(lines[1].starts_with("a     "), "text pads right: {drawn}");
    assert!(lines[1].contains("   9"), "a number pads left: {drawn}");
    assert!(
        lines[1].contains("9     ") || lines[1].ends_with('9'),
        "a mixed column stays left: {drawn}"
    );

    oslo_ui::settings::install(restore);
}

/// A rendered quantity counts as a number; a path that starts with one does not.
#[test]
fn a_quantity_reads_as_a_number_and_a_path_does_not() {
    for yes in ["4.2G", "2m30s", "-17", "1,024", "26%", "0B", "340ms"] {
        assert!(super::drawn::reads_as_a_number(yes), "{yes:?}");
    }
    for no in ["", "/dev", "2024-05 report", "n/a", "a1", "-"] {
        assert!(!super::drawn::reads_as_a_number(no), "{no:?}");
    }
}

/// With a border the table is a box: every drawn line is the same width and the header has a rule
/// under it. Without one, nothing is drawn — which is what every existing session looks like.
#[test]
fn a_border_boxes_the_table_and_none_draws_nothing() {
    let _turn = drawing();
    let restore = oslo_ui::settings::current().as_ref().clone();
    let table = Val::table(vec![
        row(&[("k", Val::Str("a".into())), ("v", Val::Int(1))]),
        row(&[("k", Val::Str("bbbb".into())), ("v", Val::Int(22))]),
    ]);

    let plain = render_display(&table);
    assert!(!plain.contains('│'), "no border by default: {plain}");
    assert_eq!(plain.lines().count(), 3, "{plain}");

    let mut settings = restore.clone();
    settings.table.border = oslo_ui::ask::Border::Rounded;
    oslo_ui::settings::install(settings);

    let boxed = render_display(&table);
    let lines: Vec<&str> = boxed.lines().collect();
    assert_eq!(lines.len(), 6, "two rules and a header rule: {boxed}");
    assert!(
        lines[0].starts_with('╭') && lines[0].ends_with('╮'),
        "{boxed}"
    );
    assert!(
        lines[2].starts_with('├') && lines[2].contains('┼'),
        "{boxed}"
    );
    assert!(
        lines[5].starts_with('╰') && lines[5].ends_with('╯'),
        "{boxed}"
    );
    let widths: Vec<usize> = lines.iter().map(|l| display_width(l)).collect();
    assert!(
        widths.windows(2).all(|w| w[0] == w[1]),
        "ragged box: {widths:?} in {boxed}"
    );

    // Not one glyph of it may reach the program on the other end of a pipe.
    assert!(!render_transport(&table).contains('│'));

    oslo_ui::settings::install(restore);
}

/// **A row is one line.** A nested cell used to spell itself out down the column: a list of three
/// pushed two extra physical lines into the table and everything below it stopped lining up.
#[test]
fn a_nested_cell_does_not_break_the_row() {
    let _turn = drawing();
    let restore = oslo_ui::settings::current().as_ref().clone();
    oslo_ui::settings::install(restore.clone());

    let table = Val::table(vec![
        row(&[
            ("id", Val::Int(1)),
            (
                "tags",
                Val::List(vec![Val::Int(1), Val::Int(2), Val::Int(3)]),
            ),
            ("meta", Val::Record(row(&[("b", Val::Int(1))]))),
        ]),
        row(&[
            ("id", Val::Int(2)),
            ("tags", Val::List(Vec::new())),
            (
                "meta",
                Val::Record(row(&[("b", Val::Int(1)), ("c", Val::Int(2))])),
            ),
        ]),
    ]);
    let drawn = render_display(&table);
    assert_eq!(
        drawn.lines().count(),
        3,
        "a header and two rows, and no more: {drawn}"
    );
    assert!(
        drawn.contains("<3 items>") && drawn.contains("<0 items>"),
        "{drawn}"
    );
    assert!(drawn.contains("<1 field>"), "the plural agrees: {drawn}");
    assert!(drawn.contains("<2 fields>"), "{drawn}");

    // Described in the drawn face, kept whole in the transport — the two faces again.
    assert!(!render_transport(&table).contains("<3 items>"));

    oslo_ui::settings::install(restore);
}

/// A filename can hold a newline and a `cmdline` can hold a tab; neither may end the row.
#[test]
fn a_control_character_in_a_cell_does_not_break_the_row() {
    let _turn = drawing();
    let restore = oslo_ui::settings::current().as_ref().clone();
    oslo_ui::settings::install(restore.clone());

    let table = Val::table(vec![row(&[
        ("name", Val::Str("first\nsecond".into())),
        ("note", Val::Str("two\twords".into())),
    ])]);
    let drawn = render_display(&table);
    assert_eq!(drawn.lines().count(), 2, "{drawn}");
    assert!(drawn.contains("first\\nsecond"), "{drawn}");
    assert!(drawn.contains("two\\twords"), "{drawn}");

    oslo_ui::settings::install(restore);
}

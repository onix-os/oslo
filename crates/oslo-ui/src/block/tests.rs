//! The three overflow policies at a known width, and the no-terminal fallback.
//!
//! Every test draws `plain()` unless it is about decoration: the rail and the colours are escape
//! sequences, and asserting against them would pin the theme rather than the layout.

use super::*;

/// A block at a fixed width, so the assertions are about the layout and not about whatever
/// terminal the tests happen to run in.
fn block(head: &str, columns: usize) -> Block {
    Block::new(head).plain().width(columns)
}

/// The shape everything else is a variation of.
#[test]
fn a_block_is_a_headline_and_its_rows() {
    let mut b = block("direnv ~/p", 80);
    b.row("changed", "PATH");
    b.row("aliases", "_b _c");
    assert_eq!(
        b.lines(),
        vec![
            "direnv ~/p".to_string(),
            "  changed PATH".to_string(),
            "  aliases _b _c".to_string(),
        ]
    );
}

/// Labels are padded to one column, so the values line up however long the labels are.
#[test]
fn the_values_start_in_the_same_column() {
    let mut b = block("head", 80);
    b.row("a", "one");
    b.row("longer", "two");
    let lines = b.lines();
    let first = lines[1].find("one").expect("value");
    let second = lines[2].find("two").expect("value");
    assert_eq!(first, second, "{lines:?}");
}

// ------------------------------------------------------------------ count

/// The default: as many items as fit, then how many did not.
///
/// This is what a directory environment needs — a Nix dev shell changes thirty-five variables and
/// the count is the information, not the thirty-fifth name.
#[test]
fn count_shows_what_fits_and_says_how_many_did_not() {
    let mut b = block("head", 30);
    b.row("added", "alpha beta gamma delta epsilon zeta");
    let line = &b.lines()[1];
    assert!(line.contains("alpha"), "{line}");
    assert!(line.contains("+"), "it must say what was dropped: {line}");
    assert!(
        !line.contains("zeta"),
        "the tail must not be printed: {line}"
    );
}

/// Everything fitting means no count at all — ` +0` would be noise on every ordinary row.
#[test]
fn count_says_nothing_when_it_all_fits() {
    let mut b = block("head", 80);
    b.row("added", "alpha beta");
    assert_eq!(b.lines()[1], "  added   alpha beta");
}

/// At least one item is always shown. A row that printed nothing and said `+35` would be worse
/// than one that runs a few cells over.
#[test]
fn count_always_shows_one_item_however_narrow() {
    let mut b = block("head", 1);
    b.row("added", "a_very_long_variable_name another");
    let line = &b.lines()[1];
    assert!(line.contains("a_very_long_variable_name"), "{line}");
    assert!(line.contains("+1"), "{line}");
}

// ------------------------------------------------------------------ ellipsis

/// One long value: keep the front, mark that there is more.
#[test]
fn ellipsis_cuts_and_marks_it() {
    let mut b = block("head", 30);
    b.row("PATH", "/nix/store/aaaaaaaaaaaa:/nix/store/bbbbbbbbbbbb");
    b.overflow(Overflow::Ellipsis);
    let line = &b.lines()[1];
    assert!(line.starts_with("  PATH    /nix/store/"), "{line}");
    assert!(line.ends_with('…'), "{line}");
    assert!(
        width_of(line) <= 30,
        "the cut must not overflow the width it was given: {line} is {}",
        width_of(line)
    );
}

/// Nothing to cut means no mark.
#[test]
fn ellipsis_leaves_a_short_value_alone() {
    let mut b = block("head", 80);
    b.row("PATH", "/usr/bin");
    b.overflow(Overflow::Ellipsis);
    assert_eq!(b.lines()[1], "  PATH    /usr/bin");
}

// ------------------------------------------------------------------ wrap

/// The content has to be read, so it continues rather than being cut.
#[test]
fn wrap_continues_on_the_next_line() {
    let mut b = block("head", 26);
    b.row("why", "the quick brown fox jumps over the lazy dog");
    b.overflow(Overflow::Wrap);
    let lines = b.lines();
    assert!(lines.len() > 2, "it should have wrapped: {lines:?}");
    for line in &lines[1..] {
        assert!(
            width_of(line) <= 26,
            "no line may overflow: {line} is {}",
            width_of(line)
        );
    }
    let joined: String = lines[1..]
        .join(" ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    assert!(joined.ends_with("lazy dog"), "nothing was lost: {joined}");
}

/// Continuation rows blank the label, so the text stays in one column.
#[test]
fn wrapped_rows_do_not_repeat_the_label() {
    let mut b = block("head", 24);
    b.row("why", "alpha beta gamma delta epsilon");
    b.overflow(Overflow::Wrap);
    let lines = b.lines();
    assert_eq!(lines[1].matches("why").count(), 1);
    for line in &lines[2..] {
        assert!(!line.contains("why"), "the label repeated: {line}");
    }
}

/// A word longer than the whole width is broken rather than left to wrap the terminal, which
/// corrupts a redraw rather than merely looking untidy.
#[test]
fn wrap_breaks_a_word_that_cannot_fit() {
    let mut b = block("head", 20);
    b.row("x", "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    b.overflow(Overflow::Wrap);
    for line in &b.lines()[1..] {
        assert!(width_of(line) <= 20, "{line} is {}", width_of(line));
    }
}

// ------------------------------------------------------------------ decoration

/// **A block in a pipe is plain text.** A rail glyph in somebody's `grep` is not a decoration.
#[test]
fn an_undecorated_block_has_no_rail_and_no_escapes() {
    let mut b = block("head", 80);
    b.row("added", "PATH");
    for line in b.lines() {
        assert!(!line.contains('│'), "{line}");
        assert!(!line.contains('\u{1b}'), "{line}");
    }
}

/// A decorated one does have a rail. The colours are the theme's business, not this file's.
#[test]
fn a_decorated_block_draws_the_rail() {
    let mut b = Block::new("head").width(80);
    b.row("added", "PATH");
    assert!(b.lines()[1].contains('│'), "{:?}", b.lines());
}

/// A headline on its own is a block. Several reports are exactly that.
#[test]
fn a_block_with_no_rows_is_just_its_headline() {
    let b = block("direnv left ~/p", 80);
    assert_eq!(b.lines().len(), 1, "a headline and nothing else");
    assert_eq!(b.lines(), vec!["direnv left ~/p".to_string()]);
}

/// A note is a row with no label — the shape a failure detail already had.
#[test]
fn a_note_is_an_unlabelled_row() {
    let mut b = block("head", 80);
    b.note("use: command not found");
    assert_eq!(b.lines()[1].trim_end(), "          use: command not found");
}

/// Width is taken once, so every row of one block agrees even at an awkward size.
#[test]
fn a_narrow_terminal_still_produces_lines() {
    for columns in 1..20 {
        let mut b = block("head", columns);
        b.row("added", "alpha beta");
        b.row("why", "some text here");
        b.overflow(Overflow::Wrap);
        assert!(b.lines().len() >= 3, "at width {columns}");
    }
}

/// The names a Lua caller writes, and a refusal for anything else — a typo must not silently pick
/// a policy the caller did not ask for.
#[test]
fn the_policy_names_are_the_ones_a_config_writes() {
    assert_eq!(Overflow::named("count"), Some(Overflow::Count));
    assert_eq!(Overflow::named("ellipsis"), Some(Overflow::Ellipsis));
    assert_eq!(Overflow::named("wrap"), Some(Overflow::Wrap));
    assert_eq!(Overflow::named("elipsis"), None);
    assert_eq!(Overflow::named(""), None);
}

fn width_of(s: &str) -> usize {
    crate::dropdown::width::display_width(s)
}

/// **A counted row must fit the width it was given, counter and all.**
///
/// The items were fitted to the whole budget and the ` +N` appended afterwards, so a row that
/// filled its width exactly overflowed by the counter and the terminal wrapped it — `+76` alone on
/// the next line, under a row that looked finished. Every width is checked because the failure
/// only appears when the items happen to land near the edge.
#[test]
fn a_counted_row_never_overflows_its_width() {
    let many: String = (0..90).map(|n| format!("VAR_{n} ")).collect();
    // From the width where one item and its counter can both fit. Below that a row deliberately
    // shows one item and overflows rather than saying nothing but `+89`; see `fit_within`.
    for columns in 24..100 {
        let mut b = block("direnv ~/p", columns);
        b.row("added", many.trim()).overflow(Overflow::Count);
        for line in b.lines() {
            assert!(
                crate::dropdown::width::display_width(&line) <= columns,
                "at width {columns}: {} cells in {line:?}",
                crate::dropdown::width::display_width(&line),
            );
        }
    }
}

/// The counter still says the truth once room has been kept for it: shown plus hidden is what
/// went in, so keeping the row inside its width cannot quietly lose an item.
#[test]
fn what_is_shown_and_what_is_counted_still_add_up() {
    let items: Vec<String> = (0..40).map(|n| format!("V{n}")).collect();
    let text = items.join(" ");
    for columns in 24..80 {
        let mut b = block("head", columns);
        b.row("added", &text).overflow(Overflow::Count);
        let line = b.lines().pop().expect("a row");
        let hidden: usize = line
            .rsplit_once(" +")
            .map(|(_, n)| n.trim().parse().expect("a number"))
            .unwrap_or(0);
        let shown = items
            .iter()
            .filter(|item| line.split_whitespace().any(|word| word == item.as_str()))
            .count();
        assert_eq!(shown + hidden, items.len(), "at width {columns}: {line:?}");
    }
}

/// Narrower than one item and its counter, a row still says something rather than nothing. The
/// overflow above is bounded by that promise, not by a missing reservation.
#[test]
fn a_row_too_narrow_for_anything_still_shows_one_item() {
    let mut b = block("head", 12);
    b.row("added", "ALPHA BETA GAMMA").overflow(Overflow::Count);
    let line = b.lines().pop().expect("a row");
    assert!(line.contains("ALPHA"), "{line:?}");
    assert!(line.ends_with("+2"), "{line:?}");
}

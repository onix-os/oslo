//! Vi mode, one keystroke at a time.
//!
//! `d2w` is three keys and one assertion here, where at a prompt it is something you have to type
//! and squint at. That difference is the whole reason the keymap is a pure function.

use super::*;

/// Run a sequence of characters in normal mode against `start`, and hand back the line.
///
/// The `|` marks the cursor, as in the buffer's own tests.
fn vi(start: &str, keys: &str) -> String {
    let (before, after) = start.split_once('|').expect("mark the cursor with |");
    let mut buf = Buffer::from_text(&format!("{before}{after}"));
    buf.set_cursor(before.chars().count());
    let mut vi = Vi {
        mode: Mode::Normal,
        ..Vi::default()
    };
    for c in keys.chars() {
        // Esc is written as `\x1b` in a key string.
        let key = if c == '\x1b' {
            Key::Cancel
        } else {
            Key::Char(c)
        };
        vi.apply(key, &mut buf);
    }
    let text = buf.text();
    let at = text.chars().take(buf.cursor()).collect::<String>().len();
    format!("{}|{}", &text[..at], &text[at..])
}

#[test]
fn character_motions_move_the_cursor() {
    assert_eq!(vi("abc|def", "h"), "ab|cdef");
    assert_eq!(vi("abc|def", "l"), "abcd|ef");
    assert_eq!(vi("abc|def", "0"), "|abcdef");
    assert_eq!(vi("abc|def", "$"), "abcdef|");
    assert_eq!(vi("  ab|c", "^"), "  |abc");
}

/// A count repeats a motion, and `0` is a motion only when no count has begun.
#[test]
fn counts_multiply_a_motion() {
    assert_eq!(vi("|abcdef", "3l"), "abc|def");
    assert_eq!(vi("abcdef|", "3h"), "abc|def");
    // `10l` is ten, not one-then-zero: the `0` is a digit once a count has started.
    assert_eq!(vi("|abcdefghijkl", "10l"), "abcdefghij|kl");
}

/// `w` splits on punctuation, `W` does not — which is what makes `W` the one that walks a path.
#[test]
fn word_motions_have_a_big_and_a_small_form() {
    assert_eq!(
        vi("|git checkout my-branch", "w"),
        "git |checkout my-branch"
    );
    assert_eq!(vi("|my-branch x", "w"), "my|-branch x");
    assert_eq!(vi("|my-branch x", "W"), "my-branch |x");
    assert_eq!(vi("git checkout| here", "b"), "git |checkout here");
    assert_eq!(
        vi("|abc def", "e"),
        "ab|c def",
        "e lands on the last character"
    );
}

/// `f` lands on the character, `t` stops one short, and `;` repeats the search.
#[test]
fn find_moves_to_a_character() {
    assert_eq!(vi("|a/b/c/d", "f/"), "a|/b/c/d");
    assert_eq!(vi("|a/b/c/d", "2f/"), "a/b|/c/d");
    assert_eq!(vi("|a/b/c/d", "t/"), "|a/b/c/d");
    assert_eq!(vi("|a/b/c/d", "f/;"), "a/b|/c/d", "; repeats the find");
    assert_eq!(vi("a/b/c|/d", "F/"), "a/b|/c/d");
}

/// The single-key edits.
#[test]
fn single_key_edits_change_the_line() {
    assert_eq!(vi("a|bc", "x"), "a|c");
    assert_eq!(vi("a|bc", "2x"), "a|");
    assert_eq!(vi("ab|c", "X"), "a|c");
    assert_eq!(vi("ab|cd", "D"), "ab|");
    assert_eq!(vi("a|bc", "rZ"), "a|Zc");
    assert_eq!(vi("a|bc", "~"), "aB|c");
}

/// Operators take a motion, and doubling one takes the whole line.
#[test]
fn operators_apply_over_a_motion() {
    assert_eq!(vi("|git checkout main", "dw"), "|checkout main");
    assert_eq!(vi("|git checkout main", "d2w"), "|main");
    assert_eq!(
        vi("|git checkout main", "2dw"),
        "|main",
        "count before or after"
    );
    assert_eq!(vi("git |checkout main", "d$"), "git |");
    assert_eq!(vi("git checkout |main", "dd"), "|");
    // **`f` is inclusive under an operator**: `df/` takes the `/` as well as what precedes it.
    assert_eq!(vi("a/b/|c/d", "df/"), "a/b/|d");
    // `t` stops one short, so the `/` survives.
    assert_eq!(vi("a/b/|c/d", "dt/"), "a/b/|/d");
    // And `de` takes the character it lands on, unlike `dw`.
    assert_eq!(vi("|abc def", "de"), "| def");
}

/// **A count on each side of an operator multiplies**, which is vi's rule and the reason the two
/// are kept apart. Sharing one field made the second overwrite the first, and made the digit in
/// `d2w` look like a motion — so it moved the cursor instead of deleting two words.
#[test]
fn counts_either_side_of_an_operator_multiply() {
    assert_eq!(vi("|one two three four five", "d2w"), "|three four five");
    assert_eq!(vi("|one two three four five", "2dw"), "|three four five");
    assert_eq!(vi("|one two three four five", "2d2w"), "|five");
    // And the operator's count does not leak into whatever comes next.
    assert_eq!(vi("|one two three four", "2dwx"), "|hree four");
}

/// **`cw` behaves as `ce`.** vi's own special case: without it, changing a word swallows the space
/// after it and you type the next word onto the previous one.
#[test]
fn cw_does_not_eat_the_following_space() {
    assert_eq!(vi("|git checkout", "cw"), "| checkout");
    assert_eq!(
        vi("|git checkout", "dw"),
        "|checkout",
        "but dw does take it"
    );
}

/// Entering insert mode from each of its keys puts the cursor in the right place.
#[test]
fn the_insert_keys_position_the_cursor() {
    let cases = [
        ("ab|cd", 'i', 2usize),
        ("ab|cd", 'a', 3),
        ("ab|cd", 'I', 0),
        ("ab|cd", 'A', 4),
    ];
    for (start, key, want) in cases {
        let (before, after) = start.split_once('|').expect("cursor");
        let mut buf = Buffer::from_text(&format!("{before}{after}"));
        buf.set_cursor(before.chars().count());
        let mut vi = Vi {
            mode: Mode::Normal,
            ..Vi::default()
        };
        vi.apply(Key::Char(key), &mut buf);
        assert_eq!(vi.mode, Mode::Insert, "{key} must enter insert");
        assert_eq!(
            buf.cursor(),
            want,
            "{key} put the cursor in the wrong place"
        );
    }
}

/// Esc leaves insert for normal and steps left, because in normal mode the cursor sits *on* a
/// character rather than after it.
#[test]
fn escape_returns_to_normal_and_steps_left() {
    let mut buf = Buffer::from_text("echo");
    let mut vi = Vi::default();
    assert_eq!(vi.mode, Mode::Insert, "a line starts in insert");
    vi.apply(Key::Cancel, &mut buf);
    assert_eq!(vi.mode, Mode::Normal);
    assert_eq!(buf.cursor(), 3, "stepped left off the end");
}

/// **Insert mode defers to the ordinary keymap**, so every readline binding still works — which is
/// what a vi user expects of a shell rather than of vi.
#[test]
fn insert_mode_passes_keys_through() {
    let mut buf = Buffer::from_text("");
    let mut vi = Vi::default();
    assert_eq!(vi.apply(Key::Char('x'), &mut buf), Outcome::Passthrough);
    assert_eq!(vi.apply(Key::Ctrl('w'), &mut buf), Outcome::Passthrough);
    assert_eq!(vi.apply(Key::Accept, &mut buf), Outcome::Passthrough);
}

/// Enter and Ctrl-C reach the caller from normal mode too, or a line could not be run without
/// first going back to insert.
#[test]
fn normal_mode_still_lets_a_line_be_run() {
    let mut buf = Buffer::from_text("ls");
    let mut vi = Vi {
        mode: Mode::Normal,
        ..Vi::default()
    };
    assert_eq!(vi.apply(Key::Accept, &mut buf), Outcome::Passthrough);
    assert_eq!(vi.apply(Key::Abort, &mut buf), Outcome::Passthrough);
}

/// `u` walks back a whole command, not a keystroke.
#[test]
fn undo_takes_back_one_command() {
    assert_eq!(vi("|git checkout main", "dwu"), "|git checkout main");
    assert_eq!(vi("ab|cd", "xxu"), "ab|d", "one x back, not both");
    // Nothing to undo leaves the line alone.
    assert_eq!(vi("a|bc", "u"), "a|bc");
}

/// `y` copies without removing, and `p` puts it back after the cursor.
#[test]
fn yank_and_put() {
    assert_eq!(vi("|abc def", "yw"), "|abc def", "y does not remove");
    assert_eq!(vi("|abc def", "ywP"), "abc |abc def");
}

/// Replace mode overwrites rather than inserting, until Esc.
#[test]
fn replace_mode_overwrites() {
    let mut buf = Buffer::from_text("abcd");
    buf.set_cursor(0);
    let mut vi = Vi {
        mode: Mode::Normal,
        ..Vi::default()
    };
    vi.apply(Key::Char('R'), &mut buf);
    assert_eq!(vi.mode, Mode::Replace);
    vi.apply(Key::Char('X'), &mut buf);
    vi.apply(Key::Char('Y'), &mut buf);
    assert_eq!(buf.text(), "XYcd", "overwrote, did not insert");
    vi.apply(Key::Cancel, &mut buf);
    assert_eq!(vi.mode, Mode::Normal);
}

/// A count typed but not used must not leak into the next command.
#[test]
fn an_abandoned_count_does_not_leak() {
    // `3` then Esc then `x` deletes one character, not three.
    assert_eq!(vi("a|bcdef", "3\x1bx"), "a|cdef");
}

/// **Esc arriving glued to the next key still means Esc.**
///
/// `ESC x` is how a terminal spells `M-x`, so an Esc pressed a few milliseconds before the next
/// key decodes as Alt. Vi mode binds no Alt chord, so it is read as Esc followed by that key —
/// without which `Esc0` typed at any speed does nothing, which is how most people leave insert
/// mode and go to the start of the line.
#[test]
fn escape_glued_to_the_next_key_still_leaves_insert_mode() {
    let mut buf = Buffer::from_text("echo hello");
    let mut vi = Vi::default();
    assert_eq!(vi.mode, Mode::Insert);
    // `Esc` and `0` arriving as one sequence.
    vi.apply(Key::Alt('0'), &mut buf);
    assert_eq!(vi.mode, Mode::Normal, "did not leave insert");
    assert_eq!(buf.cursor(), 0, "and the 0 moved to the start of the line");

    // And a whole command works the same way: Esc-glued-d, then w.
    let mut buf = Buffer::from_text("one two");
    let mut vi = Vi::default();
    vi.apply(Key::Alt('0'), &mut buf);
    vi.apply(Key::Char('d'), &mut buf);
    vi.apply(Key::Char('w'), &mut buf);
    assert_eq!(buf.text(), "two");
}

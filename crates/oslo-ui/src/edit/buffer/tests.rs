//! Every edit, checked as a pure function.
//!
//! These are the tests that make replacing a line editor tractable: none of them needs a terminal,
//! so the behaviour people have in their fingers can be pinned down before a single byte is
//! written to a screen.

use super::*;

#[test]
fn movement_and_deletion_use_extended_graphemes() {
    for grapheme in ["e\u{301}", "👍🏽", "👨‍👩‍👧‍👦", "🇳🇱", "1️⃣", "क्‍ष"]
    {
        let text = format!("a{grapheme}z");
        let mut buffer = Buffer::from_text(&text);
        buffer.move_left();
        assert_eq!(buffer.at_cursor(), Some('z'));
        buffer.move_left();
        assert_eq!(buffer.cursor(), 1, "{grapheme:?}");
        buffer.move_right();
        assert_eq!(buffer.at_cursor(), Some('z'), "{grapheme:?}");
        buffer.move_left();
        assert!(buffer.delete());
        assert_eq!(buffer.text(), "az", "{grapheme:?}");

        let mut buffer = Buffer::from_text(&text);
        buffer.move_left();
        assert!(buffer.backspace());
        assert_eq!(buffer.text(), "az", "{grapheme:?}");
        assert_eq!(buffer.cursor(), 1);
    }
}

#[test]
fn external_and_undo_cursors_are_grapheme_boundaries() {
    let text = "ae\u{301}z";
    let mut buffer = Buffer::new();
    buffer.set(text, 2);
    assert_eq!(buffer.cursor(), 1);
    buffer.set_cursor(2);
    assert_eq!(buffer.cursor(), 1);
    buffer.snapshot();
    buffer.delete();
    assert!(buffer.undo());
    assert_eq!(buffer.text(), text);
    assert_eq!(buffer.cursor(), 1);
}

#[test]
fn transpose_and_replace_do_not_split_graphemes() {
    let mut buffer = Buffer::from_text("ae\u{301}👍🏽");
    assert!(buffer.transpose());
    assert_eq!(buffer.text(), "a👍🏽e\u{301}");
    buffer.set_cursor(1);
    assert!(buffer.replace_at_cursor('x'));
    assert_eq!(buffer.text(), "axe\u{301}");
}

/// A buffer written as `"before|after"`, so a test reads like the line it describes.
fn buf(spec: &str) -> Buffer {
    let (before, after) = spec
        .split_once('|')
        .expect("a test buffer marks its cursor with |");
    let chars: Vec<char> = format!("{before}{after}").chars().collect();
    Buffer {
        cursor: before.chars().count(),
        chars,
        kill: Vec::new(),
        undo: Vec::new(),
    }
}

/// The same notation back out, so a failure says where the cursor ended up.
fn show(b: &Buffer) -> String {
    let text: String = b.chars.iter().collect();
    let at = b.chars[..b.cursor].iter().collect::<String>().len();
    format!("{}|{}", &text[..at], &text[at..])
}

#[test]
fn typing_inserts_at_the_cursor() {
    let mut b = buf("ec|ho");
    b.insert('x');
    assert_eq!(show(&b), "ecx|ho");
}

/// A paste is one splice, and lands with the cursor after it — the bug that showed up as
/// "the cursor goes one letter before the end" when this is got wrong.
#[test]
fn a_paste_leaves_the_cursor_after_it() {
    let mut b = buf("ls |");
    b.insert_str("/tmp/file");
    assert_eq!(show(&b), "ls /tmp/file|");
    let mut mid = buf("a|z");
    mid.insert_str("bc");
    assert_eq!(show(&mid), "abc|z");
}

#[test]
fn backspace_and_delete_take_from_opposite_sides() {
    let mut b = buf("ab|cd");
    assert!(b.backspace());
    assert_eq!(show(&b), "a|cd");
    assert!(b.delete());
    assert_eq!(show(&b), "a|d");
    // At the edges there is nothing to take, and that is answered rather than panicking.
    let mut start = buf("|x");
    assert!(!start.backspace());
    let mut end = buf("x|");
    assert!(!end.delete());
}

#[test]
fn motions_stop_at_the_ends() {
    let mut b = buf("|ab");
    b.move_left();
    assert_eq!(show(&b), "|ab", "left at the start stays");
    b.move_end();
    assert_eq!(show(&b), "ab|");
    b.move_right();
    assert_eq!(show(&b), "ab|", "right at the end stays");
    b.move_home();
    assert_eq!(show(&b), "|ab");
}

/// `M-b`/`M-f` treat a run of letters and digits as the word, so punctuation is crossed
/// separately. This is readline's rule, not "split on spaces".
#[test]
fn word_motions_use_alphanumeric_runs() {
    let mut b = buf("git checkout my-branch|");
    b.move_word_left();
    assert_eq!(show(&b), "git checkout my-|branch");
    b.move_word_left();
    assert_eq!(show(&b), "git checkout |my-branch");

    let mut f = buf("|git checkout my-branch");
    f.move_word_right();
    assert_eq!(show(&f), "git| checkout my-branch");
    f.move_word_right();
    assert_eq!(show(&f), "git checkout| my-branch");
}

/// **The difference that matters.** `C-w` takes a whole path; `M-DEL` takes one component.
#[test]
fn the_two_kinds_of_word_kill_differ() {
    let mut unix = buf("cat /usr/local/bin|");
    assert!(unix.kill_space_word_left());
    assert_eq!(show(&unix), "cat |", "C-w takes the whole path");

    let mut emacs = buf("cat /usr/local/bin|");
    assert!(emacs.kill_word_left());
    assert_eq!(
        show(&emacs),
        "cat /usr/local/|",
        "M-DEL takes one component"
    );
    // Again, to show it walks the path a component at a time rather than taking the rest at once.
    assert!(emacs.kill_word_left());
    assert_eq!(show(&emacs), "cat /usr/|");
}

/// `C-u` is `unix-line-discard`: to the cursor, *not* the whole line. The tail survives.
#[test]
fn kill_to_start_keeps_the_tail() {
    let mut b = buf("sudo rm |-rf /");
    assert!(b.kill_to_start());
    assert_eq!(show(&b), "|-rf /");
}

#[test]
fn kill_to_end_keeps_the_head() {
    let mut b = buf("echo hello| world");
    assert!(b.kill_to_end());
    assert_eq!(show(&b), "echo hello|");
}

/// What was killed comes back where the cursor is now, which is what makes kill-and-yank a move.
#[test]
fn yank_puts_the_last_kill_back() {
    let mut b = buf("echo |important");
    b.kill_to_end();
    b.move_home();
    assert!(b.yank());
    assert_eq!(show(&b), "important|echo ");
    // Nothing killed yet means nothing to put back, answered rather than inserting an empty run.
    assert!(!Buffer::new().yank());
}

/// `C-t` drags the character *before* the cursor forward over the one *at* it, taking the cursor
/// with it — readline's `transpose-chars`. It is **not** "swap the two behind me", which is the
/// intuition that gets this written wrong.
#[test]
fn transpose_swaps_around_the_cursor_and_at_the_end() {
    let mut mid = buf("ab|cd");
    assert!(mid.transpose());
    assert_eq!(show(&mid), "acb|d", "b is dragged forward over c");

    let mut end = buf("teh|");
    assert!(end.transpose());
    assert_eq!(show(&end), "the|");

    assert!(!buf("|").transpose(), "nothing to swap");
    assert!(!buf("a|").transpose(), "one character is not a pair");
}

#[test]
fn case_conversion_walks_the_word() {
    let mut up = buf("|echo hello");
    assert!(up.case_word(Case::Upper));
    assert_eq!(show(&up), "ECHO| hello");

    let mut title = buf("|hello world");
    assert!(title.case_word(Case::Title));
    assert_eq!(show(&title), "Hello| world");

    let mut low = buf("|LOUD quiet");
    assert!(low.case_word(Case::Lower));
    assert_eq!(show(&low), "loud| quiet");
}

/// The cursor is a character index, so a multi-byte character is one step and can never be split.
#[test]
fn multibyte_characters_are_single_steps() {
    let mut b = buf("héllo wörld|");
    b.move_word_left();
    assert_eq!(show(&b), "héllo |wörld");
    b.move_left();
    assert_eq!(show(&b), "héllo| wörld");
    assert!(b.backspace());
    assert_eq!(
        b.text(),
        "héll wörld",
        "one character removed, not one byte"
    );

    let mut cjk = buf("日本|語");
    assert!(cjk.backspace());
    assert_eq!(cjk.text(), "日語");

    let mut emoji = buf("|🎉");
    emoji.move_right();
    assert_eq!(show(&emoji), "🎉|");
    assert!(emoji.backspace());
    assert!(emoji.is_empty());
}

/// Setting a whole line — a history entry, a finder choice, a Lua key handler's answer — clamps
/// the cursor rather than trusting it.
#[test]
fn setting_a_line_clamps_the_cursor() {
    let mut b = Buffer::new();
    b.set("cargo test", 5);
    assert_eq!(show(&b), "cargo| test");
    b.set("x", 999);
    assert_eq!(show(&b), "x|", "a cursor past the end lands at the end");
}

#[test]
fn before_cursor_is_what_completion_sees() {
    let b = buf("git che|ckout");
    assert_eq!(b.before_cursor(), "git che");
    assert_eq!(b.text(), "git checkout");
}

//! The escapes, asserted rather than observed.
//!
//! Cursor bugs in a line editor are invisible until they are not: the prompt walks up the screen,
//! or a character of the last line disappears. Every one of those is an off-by-one in a sequence
//! that can be written down, so it is written down here.

use super::*;

fn at(rows: usize, cursor_row: usize, cursor_col: usize) -> At {
    At {
        rows,
        cursor_row,
        cursor_col,
    }
}

/// The first draw has nothing above it, so it does not move up — an `ESC [ 0 A` would be a move
/// of one row on some terminals, which is how the prompt starts climbing.
#[test]
fn the_first_draw_does_not_move_up() {
    let out = redraw(0, "$ ls", at(1, 0, 4));
    assert!(!out.contains("\x1b[0A"), "a zero-row move: {out:?}");
    assert!(out.starts_with("\r\x1b[K"), "{out:?}");
    assert!(out.contains("$ ls"));
    assert!(out.ends_with("\r\x1b[4C"), "{out:?}");
}

/// **The erase comes before the content, never after.** After the content the cursor may be
/// sitting in the last column with the wrap pending, and erasing from there takes back the
/// character just drawn.
#[test]
fn the_erase_happens_before_anything_is_drawn() {
    let out = redraw(0, "$ ls", at(1, 0, 4));
    let erase = out.find("\x1b[J").expect("erases");
    let content = out.find("$ ls").expect("draws");
    assert!(erase < content, "erased after drawing: {out:?}");
}

/// A redraw from a lower row walks back up to the top of the block first.
#[test]
fn a_redraw_returns_to_the_top_of_the_block() {
    let out = redraw(2, "frame", at(3, 1, 5));
    assert!(out.starts_with("\x1b[2A\r\x1b[K"), "{out:?}");
}

/// **The erase never begins at the screen's own origin.**
///
/// `ESC[J` from row 0, column 0 is an erase of the whole screen, and a terminal that keeps
/// scrollback moves what was visible into history before clearing it — so every keystroke after a
/// `Ctrl-L` pushed another copy of the half-typed prompt into the buffer. Ten keystrokes, ten
/// lines, measured in tmux. The row this erases from is stepped down to first, which is the same
/// idiom `paint.rs` and `dropdown` already use.
#[test]
fn the_erase_never_starts_at_the_screen_origin() {
    let out = redraw(0, "$ ls", at(1, 0, 4));
    let erase = out.find("\x1b[J").expect("erases");
    let down = out.find("\x1b[B").expect("steps down before erasing");
    assert!(
        down < erase,
        "the erase-to-end runs before stepping off row zero: {out:?}"
    );
    assert!(
        out[..erase].ends_with("\x1b[B\r"),
        "the erase must run from column zero of the row below: {out:?}"
    );
}

/// The cursor is placed by walking up from the *last* row, because that is where writing the
/// content leaves it.
#[test]
fn the_cursor_is_placed_from_the_bottom_of_the_block() {
    // Three rows, cursor on the first: come up two.
    let out = redraw(0, "frame", at(3, 0, 6));
    assert!(out.ends_with("\r\x1b[2A\x1b[6C"), "{out:?}");
    // Cursor already on the last row: no vertical move at all.
    let bottom = redraw(0, "frame", at(3, 2, 1));
    assert!(bottom.ends_with("\r\x1b[1C"), "{bottom:?}");
    assert!(!bottom.contains("\x1b[0A"));
}

/// Column zero needs no horizontal move, for the same reason row zero needs no vertical one.
#[test]
fn column_zero_emits_no_move() {
    let out = redraw(0, "frame", at(1, 0, 0));
    assert!(out.ends_with('\r'), "{out:?}");
    assert!(!out.contains("\x1b[0C"), "{out:?}");
}

/// A `\r` after the content, so the pending wrap cannot make the position ambiguous.
#[test]
fn the_position_is_committed_after_the_content() {
    let out = redraw(0, "$ x", at(1, 0, 3));
    let content = out.find("$ x").expect("draws");
    assert!(
        out[content..].starts_with("$ x\r"),
        "no carriage return after the content: {out:?}"
    );
}

/// The shared cursor-save slot belongs to the multiplexer. Touching it means a restore lands
/// wherever somebody else's save left the cursor.
#[test]
fn the_shared_save_slot_is_never_touched() {
    for out in [redraw(3, "f", at(4, 2, 9)), finish(1, 3)] {
        assert!(!out.contains("\x1b7"), "saves the cursor: {out:?}");
        assert!(!out.contains("\x1b8"), "restores the cursor: {out:?}");
    }
}

/// Accepting a line steps down to the bottom of the block before the newline, so the command's
/// output starts below the whole thing rather than inside it.
#[test]
fn finishing_steps_past_the_last_row() {
    assert_eq!(finish(0, 3), "\x1b[2B\r\n");
    assert_eq!(finish(2, 3), "\r\n", "already on the last row");
    assert_eq!(finish(0, 1), "\r\n", "a one-row block");
}

/// A frame that has shrunk must leave nothing of the taller one behind — which is what the erase
/// at the top of the block is for.
#[test]
fn a_shorter_frame_clears_what_the_taller_one_left() {
    let out = redraw(1, "short", at(1, 0, 5));
    assert!(
        out.contains("\x1b[J"),
        "nothing clears the rows below: {out:?}"
    );
}

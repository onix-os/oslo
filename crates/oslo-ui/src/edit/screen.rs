//! Turning a laid-out row into the escapes that put it on the terminal.
//!
//! Pure, like everything else here: [`redraw`] takes where the cursor is now and where the frame
//! wants it, and answers with a string. No terminal, so the sequences are asserted in tests rather
//! than discovered by watching a prompt march up the screen.
//!
//! # Every move is relative
//!
//! Never `ESC 7` / `ESC 8`. There is one cursor-save slot per terminal and the multiplexer hosting
//! the session shares it, so a restore lands wherever somebody else's save left the cursor. This
//! is the same rule [`crate::paint`] follows and for the same reason.
//!
//! # Why relative moves survive scrolling
//!
//! Writing a frame at the bottom of the screen makes the terminal scroll, so the row the prompt
//! started on moves up. Every position here is measured from *the cursor*, which the scroll moves
//! too — so the arithmetic stays true. An absolute row number would not.
//!
//! # The order of the erase
//!
//! `ESC [ J` erases from the cursor to the end of the screen, so it has to happen at the top-left
//! of the block **before** anything is drawn. Doing it after — at the end of the content — runs
//! into the pending wrap: on a row filled to the last column the cursor is still *in* that column,
//! and the erase would take back the character just written.

/// Where the frame wants the cursor, and how tall it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct At {
    pub rows: usize,
    pub cursor_row: usize,
    pub cursor_col: usize,
}

/// The escapes that replace the block on screen with `frame`.
///
/// `from_row` is which row of the *previous* block the cursor is sitting on — 0 for the first
/// draw, since nothing has been drawn yet and the cursor is already where the prompt goes.
pub fn redraw(from_row: usize, frame: &str, at: At) -> String {
    let mut out = String::new();

    // Back to the top-left of the block. Up first, then carriage return: a `\r` on the wrong row
    // would leave the erase below to eat a line that is not ours.
    if from_row > 0 {
        out.push_str(&format!("\x1b[{from_row}A"));
    }
    out.push('\r');
    // Everything from here down is the block's, so erasing it all and redrawing is both correct
    // and the only thing that removes a frame that has become shorter.
    out.push_str("\x1b[J");

    out.push_str(frame);

    // The content ends on the last row of the block. `\r` commits the position — with a pending
    // wrap the cursor is still on that row, and column 0 of it is unambiguous.
    out.push('\r');
    let last = at.rows.saturating_sub(1);
    if last > at.cursor_row {
        out.push_str(&format!("\x1b[{}A", last - at.cursor_row));
    }
    if at.cursor_col > 0 {
        out.push_str(&format!("\x1b[{}C", at.cursor_col));
    }
    out
}

/// The escapes that leave a finished line behind and put the cursor on a fresh row below it.
///
/// Used when Enter is pressed: the block stays on screen as part of the transcript, and the
/// command's output starts under it.
pub fn finish(cursor_row: usize, rows: usize) -> String {
    let mut out = String::new();
    // Down to the last row of the block, wherever in it the cursor happened to be.
    let last = rows.saturating_sub(1);
    if last > cursor_row {
        out.push_str(&format!("\x1b[{}B", last - cursor_row));
    }
    out.push_str("\r\n");
    out
}

#[cfg(test)]
#[path = "screen/tests.rs"]
mod tests;

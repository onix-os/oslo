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
    //
    // **Never `ESC[J` from the screen's own origin.** When the block starts at row 0 — which
    // `Ctrl-L` guarantees, since it homes the cursor and clears — an erase-to-end from there is an
    // erase of the whole screen, and a terminal keeping scrollback moves what was visible into
    // history first. One duplicate prompt line per keystroke, ten for `echo hello`.
    //
    // **And never off this row.** `ESC[B` on the last row of the screen does not move, so the
    // matching `ESC[A` landed a row *above* the block and the frame was drawn over the last line
    // of the previous command's output — invisible until the screen filled up, and then `ls` on a
    // full screen printed nothing at all. So the dodge is horizontal: clear the row, step one
    // column right, and erase from there, which is off the origin without leaving the row.
    out.push_str("\x1b[K\x1b[C\x1b[J\r");

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

/// The escapes that leave the cursor at the top of the block without touching what is drawn there.
///
/// The counterpart to [`finish`], for a line nobody typed. It clears nothing: the prompt stays on
/// screen for as long as the command runs — which is the point when the command opens a floating
/// pane beside it — and the *next* prompt is drawn over these same rows, because [`redraw`] erases
/// from the top of the block before it draws.
///
/// An earlier version cleared here instead. That took the shell off the screen the instant the key
/// was pressed and left a hole until the browser exited.
///
/// **What the command prints therefore lands on the prompt.** True, and the reason this is opt-in:
/// a key bound to something that prints wants [`finish`], which keeps the line as the record of
/// what produced the output below it.
pub fn park(cursor_row: usize) -> String {
    let mut out = String::new();
    if cursor_row > 0 {
        out.push_str(&format!("\x1b[{cursor_row}A"));
    }
    out.push('\r');
    out
}
#[cfg(test)]
#[path = "screen/tests.rs"]
mod tests;

/// A finished line's transcript: the rule, the command, the rule, over where the prompt was.
///
/// The third ending a line can have, beside [`finish`] and [`park`]. The block is cleared rather
/// than kept, because the point is that the prompt is *not* what scrolls back — see
/// [`crate::settings::Transcript`].
///
/// `unit` is repeated to `cols` rather than printed once: a rule two characters wide in the corner
/// of a terminal is not a rule. A `unit` that does not divide the width is cut, which is what any
/// rule does at the edge of a screen.
pub fn transcript(
    cursor_row: usize,
    unit: &str,
    command: &str,
    cols: usize,
    style: &str,
) -> String {
    let rule = fill_width(unit, cols);
    let painted = match style.is_empty() {
        true => rule.clone(),
        false => format!("{style}{rule}\x1b[0m"),
    };
    let mut out = park(cursor_row);
    // Everything from here down was the prompt's and is being replaced.
    out.push_str("\x1b[J");
    out.push_str(&painted);
    out.push_str("\r\n");
    // The command unstyled: it is the one line here somebody will read, and it should look the way
    // it looked while it was being typed.
    out.push_str(command);
    out.push_str("\r\n");
    out.push_str(&painted);
    out.push_str("\r\n");
    out
}

/// `unit` repeated until it reaches `cols`, cut to fit.
///
/// Counted in characters rather than bytes: a rule of `─` is three bytes a cell, and a byte count
/// would draw a third of a line.
fn fill_width(unit: &str, cols: usize) -> String {
    if unit.is_empty() || cols == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut width = 0;
    while width < cols {
        for c in unit.chars() {
            if width >= cols {
                break;
            }
            out.push(c);
            width += 1;
        }
    }
    out
}

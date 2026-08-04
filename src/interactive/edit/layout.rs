//! Where every cell of the edited row goes.
//!
//! Pure, and the reason the rest of this is worth doing. rustyline measures the prompt once, keeps
//! the number, and lays every later frame out against it — which is why oslo cannot redraw a
//! prompt, cannot change its width mid-line, cannot put an OSC marker in `$PS1`, and has to draw
//! the right prompt from inside the highlighter. Owning this function removes all four at once.
//!
//! # What it answers
//!
//! Given a prompt, the text, where the cursor is in it, a ghost hint and the terminal width:
//! how many rows the block occupies, and which row and column the cursor sits in. The caller
//! turns that into escapes with [`crate::interactive::paint`]'s discipline — reserve first, then
//! draw, never the shared save slot.
//!
//! # Cells, not characters
//!
//! A character is not a cell: `日` is two, a combining mark is none, and an emoji is two. Widths
//! come from [`crate::interactive::dropdown::display_width`], which already handles those and
//! skips SGR escapes — so a coloured prompt measures as what it prints.
//!
//! # The pending wrap
//!
//! A row filled exactly to the last column leaves the cursor *in* that column with the wrap still
//! pending; it has not moved to the next row yet. Counting it as a row consumed walks the cursor
//! one line too far and eats the line above the prompt — the same family as the bug
//! [`crate::interactive::paint`] exists to prevent. [`super::super::dropdown::physical_rows`]
//! already encodes the rule, and every count here goes through it.

use crate::interactive::dropdown::{display_width, physical_rows};

/// Everything the row is drawn from.
#[derive(Debug, Clone, Default)]
pub struct Row<'a> {
    /// The prompt, escapes and all. Measured by what it prints.
    pub prompt: &'a str,
    /// The line as it should appear — already highlighted, if it is going to be.
    pub text: &'a str,
    /// The same line without colour, for measuring and for slicing at the cursor.
    ///
    /// Kept separate rather than stripping `text`, because the highlighter is the thing that knows
    /// where its own escapes are and re-deriving that here would be a second, disagreeing parser.
    pub plain: &'a str,
    /// Cursor position as a **character** index into `plain`.
    pub cursor: usize,
    /// Ghost text shown after the line, already styled. Never counted as part of the answer.
    pub hint: &'a str,
    /// Drawn hard against the right edge of the first row, when it fits.
    pub right: &'a str,
    pub cols: usize,
}

/// Where the row's cells landed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placed {
    /// What to write, starting from column 0 of the prompt's row.
    pub text: String,
    /// Screen rows the whole block occupies, at least 1.
    pub rows: usize,
    /// Which of those rows the cursor is on, counting from the first.
    pub cursor_row: usize,
    /// Which column, counting from 0.
    pub cursor_col: usize,
}

/// How wide the prompt prints.
pub fn prompt_width(prompt: &str) -> usize {
    display_width(prompt)
}

/// Lay the row out.
pub fn place(row: &Row) -> Placed {
    let cols = row.cols.max(1);
    let prompt_cells = display_width(row.prompt);

    // Where the cursor sits, in cells from the start of the prompt. Measured from the *plain*
    // text so a colour escape between two characters cannot move it.
    let before: String = row.plain.chars().take(row.cursor).collect();
    let cursor_cells = prompt_cells + display_width(&before);

    let line_cells = prompt_cells + display_width(row.plain);
    let hint_cells = display_width(row.hint);

    let mut out = String::new();
    out.push_str(row.prompt);
    out.push_str(row.text);
    out.push_str(row.hint);

    // The right prompt goes on the first row only, and only when there is a gap to put it in.
    // One column of breathing room, so it never abuts the text — and if it does not fit it is
    // simply not drawn, which is the only honest answer for something optional.
    let used = line_cells + hint_cells;
    if !row.right.is_empty() && used < cols {
        let right_cells = display_width(row.right);
        // Strictly less, so there is always at least one blank column between the line and the
        // right prompt. Equal would let them touch, which reads as one run of text.
        if right_cells < cols - used {
            out.push_str(&" ".repeat(cols - used - right_cells));
            out.push_str(row.right);
        }
    }

    // The block is as tall as the *content* needs — the hint is part of it, since it is drawn and
    // therefore occupies cells, even though the cursor never goes there.
    let rows = physical_rows(used, cols);

    Placed {
        text: out,
        rows,
        cursor_row: cursor_cells / cols,
        cursor_col: cursor_cells % cols,
    }
}

#[cfg(test)]
#[path = "layout/tests.rs"]
mod tests;

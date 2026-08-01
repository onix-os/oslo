//! The row the prompt is drawn on, and redrawing it without the line editor's help.
//!
//! Split from [`super::prompt`] because it is a different thing: `prompt` decides what the prompt
//! *says*, this decides what is currently on screen and how to put it back. The line editor draws
//! a prompt once and never again, so anything that changes mid-line — the vi mode letter, the
//! language — has to be repainted from here.

use super::prompt::render_default_left_prompt;

/// What is currently on the prompt's row, so it can be drawn again without the line editor.
///
/// rustyline hands its prompt over once and never redraws it, and a key handler cannot ask it to —
/// `EventContext` carries a `&dyn Refresher` while `refresh_prompt_and_line` wants `&mut`. So when
/// the vi mode changes there is no way to make rustyline repaint, and a mode letter in the prompt
/// would sit there saying `I` while the cursor said otherwise.
///
/// oslo repaints the row itself instead. The highlighter runs on every line change and knows
/// everything needed — the language, the line, and where the cursor sits — so it leaves a copy
/// here, and [`repaint`] writes it out again with whatever the mode is *now*.
static ROW: std::sync::Mutex<Option<Row>> = std::sync::Mutex::new(None);

#[derive(Clone)]
struct Row {
    /// The language segment, so the prompt can be rebuilt for the right one.
    language: String,
    status: i32,
    /// Cells the prompt itself occupies, so the cursor column can be worked out from a position
    /// within the line.
    prompt_width: usize,
}

/// Record the row, for [`repaint`]. Called by the highlighter on every redraw.
pub fn note_row(language: &str, status: i32, prompt_width: usize) {
    if let Ok(mut slot) = ROW.lock() {
        *slot = Some(Row {
            language: language.to_string(),
            status,
            prompt_width,
        });
    }
}

/// Draw the prompt row again, with the vi mode as it stands now.
///
/// Returns the escapes to write, or empty when there is nothing recorded — the first prompt of a
/// session, before anything has been highlighted.
///
/// **Only the prompt is rewritten — never the line, and nothing is erased.**
///
/// That restraint is the whole of it. The first attempt cleared the row and redrew prompt *and*
/// line from the highlighter's snapshot, which broke the ghost suggestion and the completion
/// dropdown outright: rustyline draws prompt, line, *and hint*, and the snapshot has no hint in
/// it. The row came back without one while rustyline still believed it was there, so every later
/// refresh measured against a row that no longer matched.
///
/// Overwriting just the prompt is safe because a prompt's width does not change with the mode —
/// `I`, `N` and `R` are one cell each — so the line and the hint after it are untouched, and
/// rustyline's idea of the row stays true. `\r` to the start, write, `\r` and forward to wherever
/// the cursor was.
/// `line_cursor` is how many cells into the *line* the cursor sits; the prompt's own width is
/// added here. The caller gets it from the line and byte position the editor hands over.
///
/// **Not the end of the line.** Restoring to the end was the first version's bug: with the cursor
/// anywhere but the end, every mode change dragged it to the right, which looks like the block
/// jumping a slot and makes everything typed afterwards land in the wrong place.
/// Switch the language the prompt shows, answering the one now in force.
///
/// The prompt is the only place the language is written down between keystrokes, so the toggle
/// changes it here and repaints. It used to accept the line to hand control back to the read
/// loop, which cost a row and a fresh prompt every time you changed your mind about what you were
/// typing — and the thing you had already typed had to be carried across by hand.
pub fn toggle_language() -> String {
    let Ok(mut slot) = ROW.lock() else {
        return "sh".to_string();
    };
    let Some(row) = slot.as_mut() else {
        return "sh".to_string();
    };
    row.language = if row.language == "sh" { "lua" } else { "sh" }.to_string();
    row.language.clone()
}

/// The language the prompt is currently showing.
pub fn language() -> Option<String> {
    ROW.lock().ok()?.as_ref().map(|row| row.language.clone())
}

pub fn repaint(line: &str, line_cursor: usize) -> String {
    let Ok(mut slot) = ROW.lock() else {
        return String::new();
    };
    let Some(row) = slot.as_mut() else {
        return String::new();
    };
    let left = render_default_left_prompt(row.status, &row.language);
    let width = super::prompt::printed_width(&left);
    let moved = width != row.prompt_width;
    row.prompt_width = width;

    // **A row is not always one row.** Everything below is in absolute cells from the first cell of
    // the prompt, converted to a row and a column at the end.
    //
    // This used to assume a single line: `\r` then a forward move. On a line long enough to wrap,
    // `\r` homes the row the cursor happens to be on — the *last* one — so the prompt was redrawn
    // in the middle of the typed text, and the forward move was a column count larger than the
    // terminal, which clamps to the edge. That is the corruption that appears only once a line is
    // long enough, which is why short test lines never showed it.
    let cols = super::dropdown::terminal_cols().max(1);
    let cursor_cell = width + line_cursor;
    let end_cell = width + super::prompt::printed_width(line);

    let mut out = String::new();
    // Up to the row the prompt starts on. The cursor is wherever the editor last left it, which is
    // at `cursor_cell`.
    let cursor_row = cursor_cell / cols;
    if cursor_row > 0 {
        out.push_str(&format!("\x1b[{cursor_row}A"));
    }
    out.push('\r');
    out.push_str(&left);

    // The line moves with the prompt: `lua` is a cell wider than `sh`, so a width change means the
    // text has to be written again in its new place. The editor lays the row out against the width
    // it was given when the line started and will not redraw it.
    if moved {
        out.push_str(line);
        // Only to the end of the last row the text occupies; erasing further would take rows that
        // are not ours.
        out.push_str("\x1b[K");
    }

    // Back to the cursor, as a row and a column rather than a single forward move.
    let target_row = cursor_cell / cols;
    let target_col = cursor_cell % cols;
    // Where the writing above left the cursor: after the prompt, or after the line if it was
    // redrawn.
    let drawn_cell = if moved { end_cell } else { width };
    let drawn_row = drawn_cell / cols;
    if target_row > drawn_row {
        out.push_str(&format!("\x1b[{}B", target_row - drawn_row));
    } else if drawn_row > target_row {
        out.push_str(&format!("\x1b[{}A", drawn_row - target_row));
    }
    out.push('\r');
    if target_col > 0 {
        out.push_str(&format!("\x1b[{target_col}C"));
    }
    out
}

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
    /// Whether the prompt on the row is oslo's built-in one.
    ///
    /// Only the built-in prompt has anything that changes mid-line — the vi mode letter and the
    /// language — so only it is ever worth redrawing. A `$PS1`, a Lua `prompt.left` or the `PS2`
    /// continuation prompt is somebody else's text, and rebuilding the row would replace it with
    /// oslo's own.
    builtin: bool,
}

/// Record the row, for [`repaint`]. Called by the highlighter on every redraw.
pub fn note_row(language: &str, status: i32, prompt_width: usize, builtin: bool) {
    if let Ok(mut slot) = ROW.lock() {
        *slot = Some(Row {
            language: language.to_string(),
            status,
            prompt_width,
            builtin,
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

/// Return to the first screen row of an editor instance that has just ended, and clear it.
///
/// rustyline writes a newline whenever `readline_with_initial` returns, including the private
/// interrupt used to hand a finder choice back to the read loop. The next editor instance must
/// start where the old one did or accepting a choice would leave a duplicate prompt behind. The
/// old input may have wrapped, so moving up one row is not enough.
pub fn rewind_after_readline(line: &str) -> String {
    let prompt_width = ROW
        .lock()
        .ok()
        .and_then(|row| row.as_ref().map(|row| row.prompt_width))
        .unwrap_or(0);
    rewind_rows(
        prompt_width,
        super::prompt::printed_width(line),
        super::dropdown::terminal_cols().max(1),
    )
}

fn rewind_rows(prompt_width: usize, line_width: usize, cols: usize) -> String {
    // The cursor was moved to the end before rustyline wrote its newline. `end / cols` is the
    // zero-based wrapped row it occupied; one more crosses the newline back to the prompt's row.
    let rows = (prompt_width + line_width) / cols.max(1) + 1;
    format!("\x1b[{rows}A\r\x1b[J")
}

pub fn repaint(line: &str, line_cursor: usize) -> String {
    let Ok(mut slot) = ROW.lock() else {
        return String::new();
    };
    let Some(row) = slot.as_mut() else {
        return String::new();
    };
    // **Nothing to redraw unless the prompt is ours.** `repaint` rebuilds the row from the
    // built-in prompt; on a row showing `$PS1`, a Lua `prompt.left` or the `PS2` continuation
    // prompt that would overwrite the real prompt with oslo's, and — because the recorded width
    // then disagrees — also rewrite the line at the wrong column and erase the ghost hint and the
    // right prompt with it. Those prompts have no mode letter and no language segment, so there is
    // nothing a repaint could usefully change.
    // A prompt that is not oslo's own is redrawn by asking whoever owns it — see `set_renderer`.
    // Without that this returned nothing at all, which is why a Lua prompt's mode letter only
    // caught up on the next line.
    let left = if row.builtin {
        render_default_left_prompt(row.status, &row.language)
    } else {
        match variant_for(&mode_name()) {
            Some(text) => text,
            // Nothing was prepared for this mode, so leave the row alone rather than overwrite
            // somebody else's prompt with oslo's own.
            None => return String::new(),
        }
    };
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

// A prompt that is not oslo's own, drawn once for each vi mode it could show.
//
// The alternative was a callback into the read loop, but the prompt may be another program's
// output — and a callback would have run that program again on every mode change, three times a
// second while somebody holds Esc. Rendering the handful of variants once per line costs the same
// Lua either way and cannot spawn anything.
//
// Thread-local: only the loop that prepared these ever reads them, on its own thread.
thread_local! {
    static VARIANTS: std::cell::RefCell<Vec<(String, String)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Record what the prompt looks like in each vi mode, as `(mode name, text)`.
pub fn set_variants(variants: Vec<(String, String)>) {
    VARIANTS.with(|slot| *slot.borrow_mut() = variants);
}

fn variant_for(mode: &str) -> Option<String> {
    VARIANTS.with(|slot| {
        slot.borrow()
            .iter()
            .find(|(name, _)| name == mode)
            .map(|(_, text)| text.clone())
    })
}

/// The vi mode's name, or an empty string when vi mode is off.
fn mode_name() -> String {
    super::vi::mode()
        .map(|m| m.name().to_string())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::rewind_rows;

    #[test]
    fn finder_rewind_accounts_for_wrapped_input() {
        assert_eq!(rewind_rows(10, 20, 80), "\x1b[1A\r\x1b[J");
        assert_eq!(rewind_rows(10, 80, 80), "\x1b[2A\r\x1b[J");
        assert_eq!(rewind_rows(10, 160, 80), "\x1b[3A\r\x1b[J");
    }
}

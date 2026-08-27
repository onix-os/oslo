//! Prompt-row state: what the language toggle and the finder's rewind need to know about the row
//! the prompt is on.

/// The current prompt-row facts.
static ROW: std::sync::Mutex<Option<Row>> = std::sync::Mutex::new(None);

#[derive(Clone)]
struct Row {
    /// The language segment the prompt is showing, which the toggle swaps.
    language: String,
    /// Cells the prompt itself occupies, so the finder can rewind over the row it drew on.
    prompt_width: usize,
}

/// Record the row the prompt was drawn on.
pub fn note_row(language: &str, prompt_width: usize) {
    if let Ok(mut slot) = ROW.lock() {
        *slot = Some(Row {
            language: language.to_string(),
            prompt_width,
        });
    }
}

/// Switch the language the prompt shows, answering the one now in force.
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

/// Return to the first row of a finished editor block and clear it.
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
    // One row beyond the wrapped input returns across its trailing newline.
    let rows = (prompt_width + line_width) / cols.max(1) + 1;
    format!("\x1b[{rows}A\r\x1b[J")
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

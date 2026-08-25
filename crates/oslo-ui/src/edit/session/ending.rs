//! How a finished line leaves the screen.
//!
//! Split from [`super`] when that file crossed the 600-line limit, along a seam it had gained: the
//! loop draws frames while a line is being edited, and this is the one frame after it is not.

use super::screen;

/// How a finished line leaves the screen.
///
/// Three ways, and the order they are tried in is the order of how much the caller asked for:
///
/// * `erase` — a key that *is* a command, which never wanted to be seen. See
///   [`crate::editor::Answer::erase`].
/// * `oslo.transcript.rule` — the prompt is replaced by the command between two rules. See
///   [`crate::settings::Transcript`].
/// * otherwise the line stays where it was typed and the next prompt goes under it, which is what
///   every shell has always done.
///
/// A blank line takes the plain ending whatever is configured: there is no command to frame, and a
/// pair of rules around an empty row is a worse transcript than none.
pub(super) fn ending(erase: bool, line: &str, cursor_row: usize, rows: usize) -> String {
    if erase {
        return screen::park(cursor_row);
    }
    let settings = crate::settings::current();
    let rule = settings.transcript.rule.clone();
    // The rule is the switch: with none there is no transcript, whatever else is configured.
    if rule.is_empty() || line.trim().is_empty() {
        return screen::finish(cursor_row, rows);
    }
    let cols = crate::dropdown::terminal_cols();
    let was = crate::transcript::last();
    let lines: Vec<&str> = line.split('\n').collect();

    // **The renderer draws the whole row when there is one** — rule, brackets, command and the
    // colour of all three. oslo says how wide and whether the row leads with a rule; a tool whose
    // job is how things look should not be handed only the text.
    let drawn: Option<Vec<String>> = lines
        .iter()
        .enumerate()
        .map(|(at, text)| {
            crate::transcript::rendered(&crate::transcript::Row {
                text,
                cols,
                was: was.filter(|_| at == 0),
                first: at == 0,
            })
        })
        .collect();

    let block = match drawn {
        // Placed as it came, but for the indent that lines a continuation row up under the first —
        // which is oslo's because only oslo knows where the first row's rule stopped.
        Some(rendered) => screen::given(cursor_row, &rendered, cols, lines.first()),
        // Nothing installed, or a row it declined: oslo draws them all itself.
        None => {
            let painted = crate::theme::Color::parse(&settings.transcript.style)
                .map(crate::theme::Style::fg)
                .unwrap_or(crate::theme::current().prompt.aside);
            let own: Vec<String> = lines
                .iter()
                .map(|text| format!("{}{text}", settings.transcript.prefix))
                .collect();
            screen::transcript(
                cursor_row,
                &own,
                &rule,
                cols,
                &painted.open(crate::theme::depth()),
            )
        }
    };
    format!(
        "{}{block}{}",
        crate::transcript::mark(true),
        crate::transcript::mark(false)
    )
}

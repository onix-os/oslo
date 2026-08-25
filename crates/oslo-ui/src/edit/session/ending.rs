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
    let aside = crate::theme::current().prompt.aside;
    // **The header is the delegated half.** A renderer draws one line and oslo puts the rule under
    // it — see [`crate::transcript`] for why one line and not the block.
    let header = crate::transcript::rendered(line)
        .unwrap_or_else(|| format!("{}{line}", settings.transcript.prefix));
    let block = screen::transcript(
        cursor_row,
        &header,
        &rule,
        crate::dropdown::terminal_cols(),
        &aside.open(crate::theme::depth()),
    );
    format!(
        "{}{block}{}",
        crate::transcript::mark(true),
        crate::transcript::mark(false)
    )
}

//! Recording a line in the history.
//!
//! What is left of a file that used to build rustyline's `Editor` and configure it. The editor is
//! oslo's own now, so the only thing this still owns is the decision of what gets remembered.

use super::history::store::History;

/// Add a command to the history, and to the history *file*, before it runs.
///
/// Written before the command runs deliberately: a command that exits the shell, or hangs until it
/// is killed, is exactly the one you want to find in the history afterwards.
pub(super) fn remember(history: &mut History, text: &str, secret: bool) {
    if secret || ignored(text) {
        return;
    }
    history.add(text);
    publish_history(history);
}

/// Whether `oslo.history.ignore` says this line is not worth keeping.
///
/// bash's `$HISTIGNORE`, with the shell's own glob rules and matched against the whole line — so
/// `ls` means only `ls`, and `ls *` means every `ls`. The line is compared as typed, without
/// trimming: a line the user indented is `ignore_space`'s business, not this one's.
fn ignored(text: &str) -> bool {
    let patterns = oslo::ui::settings::current().history.ignore;
    patterns
        .iter()
        .any(|pattern| oslo::expand::glob::ShellPattern::from_unquoted(pattern).matches(text))
}

/// Hand the history to everything that reads it without holding the store.
pub(super) fn publish_history(history: &History) {
    super::history::publish(history.entries().to_vec());
}

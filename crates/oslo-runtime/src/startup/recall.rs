//! Which remembered lines belong to the prompt you are looking at.
//!
//! The line editor holds one history. oslo has two languages, and a shell line and a Lua line are
//! not alternatives for the same slot — offering `ls -la` at a Lua prompt suggests something that
//! cannot run. So the full set is kept here with the language each line was typed in, and the
//! editor is given only the half that matches whichever prompt is up.

use super::mode::Mode;

/// Load the lines read from the database, which returns them **oldest first**.
///
/// It was reversed here on the assumption that the query hands back newest first. It does not —
/// `History::recent` reverses its own `ORDER BY id DESC` before returning — so this put the whole
/// recalled set backwards: the first Up offered the oldest command in the database instead of the
/// most recent one.
pub(super) fn seed_history(entries: impl Iterator<Item = (String, String)>) {
    oslo_ui::recall::seed(entries.collect());
}

/// Fill **both** readers of the history from the profile database.
///
/// `recall` decides what the Up arrow and the ghost offer; the editor's own list is what the
/// `history` builtin prints. They are filled from one query because they are two views of one
/// record — the store. `$HISTFILE` is written for other programs and never read back, so a shell
/// with no file has exactly the same history as one with a file; see [`super::history::store`].
pub(super) fn seed_from_store(history: &mut super::history::store::History, max: usize) {
    let Some(db) = oslo_base::track::store() else {
        return;
    };
    let entries = db.recent(max.max(1));
    seed_history(entries.iter().map(|e| (e.line.clone(), e.mode.clone())));
    history.seed(entries.iter().map(|e| e.line.clone()));
}

/// Remember a line typed this session, so a later language switch still finds it.
pub(super) fn remember_history(line: &str, mode: Mode) {
    oslo_ui::recall::remember(line, mode.name());
}

// There was a `load_history_for` here that cleared the editor's history and refilled it with one
// language. It is gone, and both reasons matter.
//
// It **corrupted `$HISTFILE`**: the editor counts entries added since the last clear and appends
// exactly those on the next `append_history`, so re-seeding N lines made the following command
// write all N to the file again — once at startup and again on every language toggle.
//
// And it was never the right mechanism: the language can change mid-line, from a key handler that
// cannot reach the editor at all. Everything that must follow the language — the ghost suggestion,
// the Up/Down walk, history expansion — reads [`oslo_ui::recall`] directly instead, which
// is correct the instant the prompt changes. The editor's own history stays the complete record,
// which is what `$HISTFILE` should receive.

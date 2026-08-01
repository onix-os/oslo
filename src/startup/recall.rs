//! Which remembered lines belong to the prompt you are looking at.
//!
//! The line editor holds one history. oslo has two languages, and a shell line and a Lua line are
//! not alternatives for the same slot — offering `ls -la` at a Lua prompt suggests something that
//! cannot run. So the full set is kept here with the language each line was typed in, and the
//! editor is given only the half that matches whichever prompt is up.

use super::mode::Mode;
use super::repl::Repl;

/// Load the lines read from the database, newest first as the query returns them.
pub(super) fn seed_history(entries: impl Iterator<Item = (String, String)>) {
    // Reversed: the database hands back newest first, history reads oldest first.
    let mut all: Vec<(String, String)> = entries.collect();
    all.reverse();
    oslo::interactive::recall::seed(all);
}

/// Remember a line typed this session, so a later language switch still finds it.
pub(super) fn remember_history(line: &str, mode: Mode) {
    oslo::interactive::recall::remember(line, mode.name());
}

/// Refill the editor's history with the lines belonging to `mode`, and nothing else.
///
/// The suggestion no longer depends on this — it reads the same set directly, so it is right the
/// instant the language changes. This keeps the *editor's* own recall honest: the arrow keys and
/// `Ctrl-R` walk the editor's history, and it should hold what the prompt is reading.
pub(super) fn load_history_for(rl: &mut Repl, mode: Mode) {
    let _ = rl.clear_history();
    for line in oslo::interactive::recall::for_language(mode.name()) {
        let _ = rl.add_history_entry(&line);
    }
}

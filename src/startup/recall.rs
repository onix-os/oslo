//! Which remembered lines belong to the prompt you are looking at.
//!
//! The line editor holds one history. oslo has two languages, and a shell line and a Lua line are
//! not alternatives for the same slot — offering `ls -la` at a Lua prompt suggests something that
//! cannot run. So the full set is kept here with the language each line was typed in, and the
//! editor is given only the half that matches whichever prompt is up.

use super::mode::Mode;
use super::repl::Repl;
use std::sync::Mutex;

/// Every remembered line with the language it was typed in, newest last.
///
/// Held here rather than left in the editor because the editor can only hold *one* history, and
/// oslo has two: a shell line and a Lua line are not alternatives for the same slot. Recalling
/// `ls -la` at a Lua prompt offers something that cannot run.
static REMEMBERED: Mutex<Vec<(String, String)>> = Mutex::new(Vec::new());

/// Load the lines read from the database.
pub(super) fn seed_history(entries: impl Iterator<Item = (String, String)>) {
    if let Ok(mut all) = REMEMBERED.lock() {
        all.clear();
        // The database hands them back newest first; history reads oldest first.
        all.extend(entries);
        all.reverse();
    }
}

/// Remember a line typed this session, so a later language switch still finds it.
pub(super) fn remember_history(line: &str, mode: Mode) {
    if let Ok(mut all) = REMEMBERED.lock() {
        all.push((line.to_string(), mode.name().to_string()));
    }
}

/// Refill the editor's history with the lines belonging to `mode`, and nothing else.
///
/// Called when the language changes as well as at startup: the arrow keys, `Ctrl-R` and the ghost
/// suggestion all read the editor's history, so filtering it here is what makes every one of them
/// agree about which language is being typed.
pub(super) fn load_history_for(rl: &mut Repl, mode: Mode) {
    let _ = rl.clear_history();
    let Ok(all) = REMEMBERED.lock() else {
        return;
    };
    for (line, entry_mode) in all.iter() {
        if entry_mode == mode.name() {
            let _ = rl.add_history_entry(line);
        }
    }
}

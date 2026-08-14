//! Where the shell reaches a hook, without knowing that Lua exists.
//!
//! # Why this is not in the Lua layer, where it was
//!
//! The editor, the executor and the `cd` builtin all fire hooks, and they used to do it by calling
//! `lua::engine::fire_at_here` directly. That single fact pinned the whole Lua layer *underneath*
//! them — and the Lua layer also holds `lua::api`, which needs to sit *above* them, because it is
//! how a config reaches the shell. One module cannot be on both sides of everything else.
//!
//! So the dependency is inverted here. This registry knows the *moments* ([`at`]) and nothing about
//! what happens at one; the runtime [`install`]s four function pointers at startup and the calls
//! find their way up. Before that — a script, `sh -c`, any test that never starts an interpreter —
//! [`watched`] answers `false` and every fire is a single relaxed load and a return, which is what
//! it already cost when nothing was attached.
//!
//! # What it does depend on
//!
//! [`Value`], for the two shapes that carry more than strings. That is a downward edge: the Lua
//! *interpreter* is the bottom of this stack and the Lua *API* is the top, and only the first is
//! named here. The same edge already exists as `ShellError::Lua`.

use oslo_lua::value::{Table, Value};
use std::sync::OnceLock;

/// The moments something on a hot path has to ask about, by index.
///
/// Named because the check is in a loop that runs per keystroke, per mode change or per prompt —
/// somewhere a string comparison against twenty names would be real work for nothing.
///
/// **The indices are the contract between this crate and the runtime**, which holds the table of
/// names these point into. `lua::api::hooks` has a test asserting every one of them still names
/// what it is called after; that test is what stops the two drifting now that they are apart.
pub mod at {
    pub const PRE_CMD: usize = 0;
    pub const POST_CMD: usize = 1;
    pub const PRE_CHANGE_DIR: usize = 2;
    pub const POST_CHANGE_DIR: usize = 3;
    pub const PRE_PROMPT: usize = 4;
    pub const POST_PROMPT: usize = 5;
    pub const PRE_MODE_CHANGE: usize = 6;
    pub const POST_MODE_CHANGE: usize = 7;
    pub const HISTORY_OPEN: usize = 8;
    pub const HISTORY_CLOSE: usize = 9;
    pub const HISTORY_SELECT: usize = 10;
    pub const COMPLETION_START: usize = 11;
    pub const COMPLETION_CANCEL: usize = 12;
    pub const COMPLETION_SELECT: usize = 13;
    pub const JOB_FINISH: usize = 14;
    pub const TIME_REPORT: usize = 15;
    /// The one moment that had no constant, which is why its fire site named it by *alias* — and
    /// so looked up a key nothing is ever stored under.
    pub const COMMAND_NOT_FOUND: usize = 16;
    pub const IDLE_TIMEOUT: usize = 17;
    /// How a report the shell was about to print should look.
    pub const ON_REPORT: usize = 18;
    /// Just before a finished line is written down. May replace what is recorded, or refuse it.
    pub const PRE_RECORD: usize = 19;
    pub const ON_EXIT: usize = 20;
    pub const ON_KEY: usize = 21;

    /// Asked to *do* the crypto for a store the configuration marked as hook-backed. The answer is
    /// the ciphertext; `nil` means "not mine" and the next handler is asked.
    pub const SECRET_ENCRYPT: usize = 22;
    /// The same, the other way: ciphertext in, the value out.
    pub const SECRET_DECRYPT: usize = 23;
    /// A secret is about to be read or written. Told the store, the name and which it is — never
    /// the value.
    pub const PRE_SECRET_ACCESS: usize = 24;
    pub const POST_SECRET_ACCESS: usize = 25;

    /// One process of a job ended. Told the pid, the job it belonged to and how it went.
    ///
    /// **One per process, where [`JOB_FINISH`] is one per job.** A pipeline of three is three of
    /// these and one of those, which is the difference a plugin watching a particular child needs
    /// and could not previously get.
    pub const PROCESS_EXIT: usize = 26;
    /// A job changed state: running, stopped, or ended.
    ///
    /// The transition rather than the destination — `from` and `to` — because "it stopped" and "it
    /// was already stopped" are different things to a status line.
    pub const JOB_STATE: usize = 27;
}

/// The four ways a hook is reached, supplied by whoever can actually run one.
///
/// Function pointers rather than a trait object: there is exactly one implementation, it is
/// installed once, and the read is on the keystroke path.
#[derive(Clone, Copy)]
pub struct Dispatch {
    /// Whether anything is attached — asked first by everything below, so a moment nobody cares
    /// about costs one relaxed load.
    pub watched: fn(usize) -> bool,
    /// Tell a hook something happened. Fields are `(name, value)` pairs because every caller has
    /// strings; the ones that do not use [`answer_hook_with`].
    pub fire: fn(usize, &[(&str, &str)]),
    /// Ask, and take the first handler's answer.
    pub answer: fn(usize, Vec<Value>) -> Option<Value>,
    /// Ask for a *status*, which is what `on-command-not-found` answers with.
    pub ask: fn(usize, Vec<Value>) -> Option<i32>,
}

static DISPATCH: OnceLock<Dispatch> = OnceLock::new();

/// Hand the registry a way to reach handlers. Called once, by the runtime, at startup.
///
/// A second call is ignored rather than refused: a test that starts an interpreter twice in one
/// process is doing nothing wrong, and the two would install the same pointers anyway.
pub fn install(dispatch: Dispatch) {
    let _ = DISPATCH.set(dispatch);
}

/// Whether the hook at `index` has anything attached.
///
/// `false` before anything is installed, which is the ordinary state for a script or `sh -c` and
/// the reason nothing below has to know whether an interpreter exists.
pub fn watched(index: usize) -> bool {
    DISPATCH.get().is_some_and(|d| (d.watched)(index))
}

/// Tell a hook something happened, from wherever the shell happens to be.
pub fn fire_at_here(index: usize, fields: &[(&str, &str)]) {
    if let Some(dispatch) = DISPATCH.get() {
        (dispatch.fire)(index, fields);
    }
}

/// Fire a hook that may *answer*, and hand back the first answer given.
pub fn answer_hook_with(index: usize, args: Vec<Value>) -> Option<Value> {
    let dispatch = DISPATCH.get()?;
    (dispatch.answer)(index, args)
}

/// Fire a hook that answers with a status.
pub fn ask_hook_here(index: usize, args: Vec<Value>) -> Option<i32> {
    let dispatch = DISPATCH.get()?;
    (dispatch.ask)(index, args)
}

/// A table of named values, for a hook whose payload is more than strings.
///
/// Here rather than on the engine so that a caller building one does not have to be able to see the
/// engine — which was the whole problem.
pub fn fields(named: &[(&str, Value)]) -> Value {
    let mut table = Table::new();
    for (name, value) in named {
        table.set(Value::str(*name), value.clone());
    }
    Value::table(table)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **Nothing installed is a working state, not an error.** A script and `sh -c` never start an
    /// interpreter, and every one of these sits on a path they take.
    ///
    /// Guarded on `DISPATCH` being empty rather than calling `install`: this asserts what a fresh
    /// process does, and an installation would leak into every other test in this binary — the
    /// slot is process-wide and set once.
    #[test]
    fn every_entry_point_is_quiet_before_anything_is_installed() {
        if DISPATCH.get().is_some() {
            return;
        }
        assert!(!watched(at::PRE_CMD));
        fire_at_here(at::POST_CMD, &[("a", "b")]);
        assert!(answer_hook_with(at::ON_KEY, Vec::new()).is_none());
        assert!(ask_hook_here(at::COMMAND_NOT_FOUND, Vec::new()).is_none());
    }

    #[test]
    fn a_payload_is_a_table_of_what_it_was_given() {
        let built = fields(&[("from", Value::str("/a")), ("to", Value::str("/b"))]);
        let Value::Table(table) = built else {
            panic!("a hook payload is a table");
        };
        let table = table.borrow();
        assert!(matches!(table.get(&Value::str("from")), Value::Str(s) if &*s == "/a"));
        assert!(matches!(table.get(&Value::str("to")), Value::Str(s) if &*s == "/b"));
    }
}

//! oslo's own machinery behind the native editor's [`Assist`].
//!
//! The bridge between [`oslo::interactive::edit`], which knows how to edit and draw a line, and
//! everything the shell already had — the syntax highlighter, the history, the environment. None
//! of that is new; it was reachable all along behind rustyline's traits, which is why this file is
//! mostly plumbing rather than logic.
//!
//! # Why the right prompt is not here
//!
//! Because it no longer has to be. Under rustyline it was smuggled out of the *highlighter*, since
//! that was the only place a cursor move did not confuse the layout. The native editor takes it as
//! an argument, which is what it always should have been.

use oslo::Environment;
use oslo::interactive::edit::session::Assist;
use oslo::interactive::{highlight, marks};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

/// What the shell plugs into an editing session.
pub struct ShellAssist {
    env: Arc<Mutex<Environment>>,
    /// History, newest last — the same order the editor's own store keeps.
    history: Vec<String>,
    /// How many steps back into history the walk has gone. `0` is the line being composed.
    back: usize,
    /// What was on the line when the walk started, so coming back out restores it rather than
    /// blanking it. oslo has always promised this; it is the reason a walk is not destructive.
    composing: Option<String>,
}

impl ShellAssist {
    pub fn new(env: Arc<Mutex<Environment>>, history: Vec<String>) -> ShellAssist {
        ShellAssist {
            env,
            history,
            back: 0,
            composing: None,
        }
    }

    /// Start a fresh line: the walk position resets, or Up would resume where the last line left
    /// off and appear to skip entries.
    pub fn begin(&mut self) {
        self.back = 0;
        self.composing = None;
    }
}

impl Assist for ShellAssist {
    fn highlight(&mut self, line: &str) -> String {
        if line.is_empty() {
            return String::new();
        }
        let (path, builtins, functions) = {
            let Ok(env) = self.env.lock() else {
                return line.to_string();
            };
            let path = env.get_var("PATH").unwrap_or_default().to_string();
            // Snapshotted rather than queried per word: the closures below run once per command
            // word and would each take the lock again while this one is still held.
            let builtins: HashSet<String> = env.builtin_names().map(str::to_string).collect();
            let functions: HashSet<String> = env
                .get_functions()
                .keys()
                .chain(env.get_aliases().keys())
                .cloned()
                .collect();
            (path, builtins, functions)
        };
        let is_builtin = |name: &str| builtins.contains(name);
        let is_function = |name: &str| functions.contains(name);
        let ctx = highlight::Context {
            path: &path,
            is_builtin: &is_builtin,
            is_function: &is_function,
            // A line long enough for the syscalls to add up is one nobody is reading the colours
            // of. See `highlight::MAX_PATH_CHECKS`.
            check_paths: line.len() <= 512,
        };
        // `OSC 133;B` first, so it lands between the prompt and the typed text, which is where it
        // means anything. It prints nothing, so it costs no cells.
        let mut painted = marks::input_start();
        painted.push_str(&highlight::paint(line, &ctx));
        painted
    }

    fn history_prev(&mut self, line: &str) -> Option<String> {
        let entry = self.history.iter().rev().nth(self.back)?.clone();
        if self.back == 0 {
            self.composing = Some(line.to_string());
        }
        self.back += 1;
        Some(entry)
    }

    fn history_next(&mut self) -> Option<String> {
        match self.back {
            0 => None,
            // Out the far end of the walk: the line being composed comes back.
            1 => {
                self.back = 0;
                self.composing.take()
            }
            _ => {
                self.back -= 1;
                self.history.iter().rev().nth(self.back - 1).cloned()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assist(entries: &[&str]) -> ShellAssist {
        ShellAssist::new(
            Arc::new(Mutex::new(Environment::new())),
            entries.iter().map(|e| e.to_string()).collect(),
        )
    }

    /// Up walks backwards from the newest, and Down walks back out — restoring what was typed.
    #[test]
    fn the_history_walk_is_not_destructive() {
        let mut a = assist(&["first", "second"]);
        assert_eq!(a.history_prev("draft").as_deref(), Some("second"));
        assert_eq!(a.history_prev("draft").as_deref(), Some("first"));
        assert_eq!(a.history_next().as_deref(), Some("second"));
        assert_eq!(
            a.history_next().as_deref(),
            Some("draft"),
            "the composed line must come back, not a blank"
        );
        assert_eq!(a.history_next(), None, "and then there is nowhere to go");
    }

    /// A new line resets the walk. Without this, Up on the next prompt resumes where the last one
    /// stopped and looks like it skipped entries.
    #[test]
    fn a_new_line_starts_the_walk_over() {
        let mut a = assist(&["one", "two"]);
        a.history_prev("");
        a.begin();
        assert_eq!(a.history_prev("").as_deref(), Some("two"));
    }

    /// Running off the end leaves the line alone rather than clearing it.
    #[test]
    fn walking_past_the_oldest_entry_stops() {
        let mut a = assist(&["only"]);
        assert_eq!(a.history_prev("x").as_deref(), Some("only"));
        assert_eq!(a.history_prev("x"), None);
    }

    /// An empty line paints to nothing at all — no escapes, so the layout measures zero cells.
    #[test]
    fn an_empty_line_paints_to_nothing() {
        assert_eq!(assist(&[]).highlight(""), "");
    }
}

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
use oslo::interactive::{OsloHelper, dropdown, highlight, marks, recall, settings};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

/// What the shell plugs into an editing session.
pub struct ShellAssist<'a> {
    env: Arc<Mutex<Environment>>,
    /// The completion and hinting machinery, borrowed rather than rebuilt: it carries the
    /// frecency table and the command index, and a second copy would rank differently.
    helper: Option<&'a OsloHelper>,
    /// How wide the prompt prints.
    ///
    /// The **real** width. Under rustyline the dropdown had to guess it by rendering a default
    /// prompt, because the editor never told anyone where the line started — so a custom prompt
    /// put the menu in the wrong column. Here it is simply known.
    prompt_cols: usize,
    /// History, newest last — the same order the editor's own store keeps.
    history: Vec<String>,
    /// How many steps back into history the walk has gone. `0` is the line being composed.
    back: usize,
    /// What was on the line when the walk started, so coming back out restores it rather than
    /// blanking it. oslo has always promised this; it is the reason a walk is not destructive.
    composing: Option<String>,
}

impl<'a> ShellAssist<'a> {
    pub fn new(
        env: Arc<Mutex<Environment>>,
        history: Vec<String>,
        helper: Option<&'a OsloHelper>,
        prompt_cols: usize,
    ) -> ShellAssist<'a> {
        ShellAssist {
            env,
            helper,
            prompt_cols,
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

impl Assist for ShellAssist<'_> {
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

    /// The ghost suggestion, from `oslo.suggest.sources` in order.
    ///
    /// Only at the end of the line: a suggestion is text that *continues* what you have typed, and
    /// appending it after a cursor sitting mid-line would be a claim about the wrong position.
    fn hint(&mut self, line: &str, cursor: usize) -> Option<String> {
        let helper = self.helper?;
        if line.is_empty() || cursor < line.chars().count() {
            return None;
        }
        let pos = line.len();
        for source in settings::current().suggest.sources {
            let found = match source {
                // oslo's own set, not the editor's: `recall` is language-filtered and knows which
                // directory you are standing in, so `cargo run --ex` answers with this project's
                // example. The editor's flat history can only ever know the one list.
                settings::Source::History => recall::suggest(line),
                settings::Source::Completion => helper.command_hint(line, pos),
                settings::Source::Path => helper.path_hint(line, pos),
            };
            if let Some(text) = found {
                let theme = oslo::interactive::theme::current();
                return Some(
                    theme
                        .syntax
                        .autosuggestion
                        .paint(&text, oslo::interactive::theme::depth()),
                );
            }
        }
        None
    }

    /// Tab. Runs the whole interaction — the dropdown draws itself and takes its own keys — and
    /// answers with the line it produced.
    fn complete(&mut self, line: &str, cursor: usize, _back: bool) -> Option<(String, usize)> {
        let helper = self.helper?;
        // The dropdown works in bytes; the editor's cursor is in characters.
        let pos: usize = line.chars().take(cursor).map(char::len_utf8).sum();
        let (start, candidates) = helper.candidates(line, pos);
        if candidates.is_empty() {
            return None;
        }

        let chosen = if candidates.len() == 1 {
            candidates.into_iter().next()?
        } else {
            let indent = self.prompt_cols + dropdown::visible_len(&line[..start]);
            dropdown::DropdownMenu::select_interactive(candidates, indent, &line[start..pos])?
        };
        helper.record_accepted(&chosen);

        let mut out = String::with_capacity(line.len() + chosen.replacement.len());
        out.push_str(&line[..start]);
        out.push_str(&chosen.replacement);
        let at = out.chars().count();
        out.push_str(&line[pos..]);
        Some((out, at))
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

    fn assist(entries: &[&str]) -> ShellAssist<'static> {
        ShellAssist::new(
            Arc::new(Mutex::new(Environment::new())),
            entries.iter().map(|e| e.to_string()).collect(),
            None,
            0,
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

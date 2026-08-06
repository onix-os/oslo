//! The interactive line editor: completion, hints, colouring and multi-line input.
//!
//! [`OsloHelper`] is the rustyline `Helper`. The trait implementations here are deliberately thin
//! — each one delegates to a module that can be called directly from a test, because none of this
//! behaviour was reachable without a pty and that is precisely why it was all wrong.

pub mod abbr;
pub mod ask;
/// A headline and a rail of labelled rows — the shape every report the shell prints has.
pub mod block;
pub mod command_index;
pub mod completion;
pub mod dropdown;
pub mod edit;
pub mod editor;
pub mod finder;
pub mod frecency_store;
pub mod highlight;
mod hinting;
pub mod keys;
pub mod marks;
pub mod matching;
pub mod paint;
pub mod prompt;
pub mod query;
pub mod recall;
/// `on-report` — letting a config draw what the shell was going to draw.
pub mod report;
pub mod row;
pub mod scanner;
pub mod settings;
pub mod spec;
pub mod syntax;
pub mod term;
pub mod theme;
pub mod vi;
pub mod words;

#[cfg(test)]
mod tests;

pub use command_index::invalidate as invalidate_command_cache;
pub use syntax::{DEFAULT_PS2, InputStatus};
pub use words::{Quote, Word, current_word, extract_current_word};

use crate::env::Environment;
use dropdown::CompletionCandidate;
use frecency_store::FrecencyStore;
use spec::SpecRegistry;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

/// Completion kinds worth remembering.
///
/// Filenames are deliberately absent: they are unbounded and mostly one-off, and letting them
/// into the table would drown the command ranking it exists to serve.
const RANKED_KINDS: &[&str] = &["command", "builtin", "subcommand"];

pub struct OsloHelper {
    env: Arc<Mutex<Environment>>,
    spec_registry: SpecRegistry,
    frecency: FrecencyStore,
    /// Whether Tab may take the terminal over to draw the dropdown.
    ///
    /// Off when there is no terminal, which is what makes `complete` callable from a test: the
    /// dropdown would otherwise swallow every ambiguous candidate list and return nothing.
    menu: bool,
    /// Whether the editor itself accumulates continuation lines.
    ///
    /// See [`OsloHelper::set_editor_multiline`].
    editor_multiline: bool,
}

impl OsloHelper {
    /// Build the helper for `env`.
    ///
    /// Two side effects hang off whether `env` belongs to an interactive shell: the dropdown
    /// takes the terminal over, and the frecency table is read from and appended to a file in
    /// `$HOME`. `$-` is the signal rather than `isatty`, because `cargo test` inherits a terminal
    /// on stdin and a test must not write to the user's home directory.
    pub fn new(env: Arc<Mutex<Environment>>) -> Self {
        let interactive = env.lock().unwrap().interactive();
        let frecency = if interactive {
            FrecencyStore::load(FrecencyStore::default_path())
        } else {
            FrecencyStore::in_memory()
        };
        Self {
            env,
            spec_registry: SpecRegistry::new(),
            frecency,
            menu: interactive,
            editor_multiline: true,
        }
    }

    pub fn set_menu(&mut self, enabled: bool) {
        self.menu = enabled;
    }

    /// Whether unterminated input is continued inside the editor.
    ///
    /// With it on (the default), `validate` answers `Incomplete` and rustyline keeps reading into
    /// the same buffer. That fixes the three R9.1 failures on its own, but rustyline draws no
    /// prompt on a continuation row and cannot be made to: it computes the cursor's column from
    /// the raw buffer, so any prefix the highlighter added would put the cursor in the wrong
    /// place. A caller that wants a real PS2 turns this off and drives the loop itself, calling
    /// [`OsloHelper::input_status`] after each line and re-reading with
    /// [`OsloHelper::continuation_prompt`] while the answer is [`InputStatus::Incomplete`].
    pub fn set_editor_multiline(&mut self, enabled: bool) {
        self.editor_multiline = enabled;
    }

    /// Whether `buffer` is a complete program, needs another line, or is a syntax error.
    pub fn input_status(&self, buffer: &str) -> InputStatus {
        syntax::classify(buffer)
    }

    /// The prompt to show while reading a continuation line: `$PS2`, or `> `.
    pub fn continuation_prompt(&self) -> String {
        self.env
            .lock()
            .unwrap()
            .get_var("PS2")
            .map(str::to_string)
            .unwrap_or_else(|| DEFAULT_PS2.to_string())
    }

    /// Count the commands in an accepted line towards their frecency.
    ///
    /// **Whose job it is to call this depends on who assembles the command**, and getting that
    /// wrong is how multi-line commands stopped counting (PLAN C10). `validate` calls it only
    /// while [`OsloHelper::set_editor_multiline`] is on, because that is the mode in which
    /// rustyline itself accumulates continuation lines and `validate` really does see the whole
    /// program. With it off — which is what the REPL does, so that `PS2` can be drawn — `validate`
    /// sees each line separately and answers `Incomplete` for every one but the last, so the loop
    /// that built the buffer calls this instead. The comment that used to be here claimed
    /// `validate` was "the one place the editor sees a line the user committed to"; it had been
    /// false since the REPL turned editor multi-line off, and `for i in …; do git …; done` never
    /// taught the ranker anything about `git`.
    pub fn record_command_use(&self, line: &str) {
        for name in words::command_words(line) {
            self.frecency.record(&name);
        }
    }

    /// This command's frecency score. Exposed for tests and for ranking outside the helper.
    pub fn frecency_score(&self, name: &str) -> f64 {
        self.frecency.score(name)
    }

    /// Paint a line with the current theme, for drawing it as it is typed.
    ///
    /// Moved here from the editor's side of the bridge when rustyline went: it needs the
    /// environment, and this is what holds it.
    pub fn paint(&self, line: &str) -> String {
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
        highlight::paint(line, &ctx)
    }

    /// The ghost suggestion for `line`, in `oslo.suggest.sources` order, as plain text.
    ///
    /// Only at the end of the line: a suggestion *continues* what you have typed, so offering one
    /// for a cursor sitting mid-line would be a claim about the wrong position.
    pub fn suggest(&self, line: &str, pos: usize) -> Option<String> {
        if line.is_empty() || pos < line.len() {
            return None;
        }
        // The `suggest` feature, which is a runtime mask over the configured sources rather than a
        // second way to configure them: turning it back on restores whatever `oslo.suggest` said.
        if !crate::feature::on(crate::feature::at::SUGGEST) {
            return None;
        }
        for source in settings::current().suggest.sources {
            let found = match source {
                // oslo's own set, not a flat editor history: `recall` is language-filtered and
                // knows which directory you are standing in, so `cargo run --ex` answers with
                // this project's example.
                settings::Source::History => recall::suggest(line),
                settings::Source::Completion => self.command_hint(line, pos),
                settings::Source::Path => self.path_hint(line, pos),
            };
            if found.is_some() {
                return found;
            }
        }
        None
    }

    /// Paint a ghost suggestion in the autosuggestion colour.
    ///
    /// Separate from the suggestion itself so that what is *inserted* when you accept it is plain
    /// text: escapes in the line would end up in the history and in what runs.
    pub fn paint_hint(&self, hint: &str) -> String {
        let theme = theme::current();
        theme.syntax.autosuggestion.paint(hint, theme::depth())
    }

    /// Complete the word at `pos`, recording an unambiguous answer as an acceptance.
    ///
    /// One candidate is inserted without asking, and that *is* an acceptance — so it feeds the
    /// frecency ranking exactly as choosing from the menu does.
    pub fn complete_word(&self, line: &str, pos: usize) -> (usize, Vec<CompletionCandidate>) {
        let (start, candidates) = self.candidates(line, pos);
        if let [only] = candidates.as_slice() {
            self.record_accepted(only);
        }
        (start, candidates)
    }

    /// Note an accepted candidate for frecency ranking.
    ///
    /// `pub` because the native editor drives completion itself and must record the same
    /// acceptance rustyline's path did — otherwise ranking would quietly stop learning the
    /// moment the native editor was switched on.
    pub fn record_accepted(&self, candidate: &CompletionCandidate) {
        if candidate
            .kind
            .as_deref()
            .is_some_and(|k| RANKED_KINDS.contains(&k))
        {
            self.frecency.record(&candidate.display);
        }
    }
}

impl OsloHelper {}

//! The interactive line editor: completion, hints, colouring and multi-line input.
//!
//! [`OsloHelper`] is the rustyline `Helper`. The trait implementations here are deliberately thin
//! — each one delegates to a module that can be called directly from a test, because none of this
//! behaviour was reachable without a pty and that is precisely why it was all wrong.

pub mod command_index;
mod completion;
pub mod dropdown;
pub mod frecency_store;
pub mod highlight;
mod hinting;
pub mod marks;
pub mod prompt;
pub mod spec;
pub mod syntax;
pub mod words;

#[cfg(test)]
mod tests;

pub use command_index::invalidate as invalidate_command_cache;
pub use syntax::{DEFAULT_PS2, InputStatus};
pub use words::{Quote, Word, current_word, extract_current_word};

use crate::env::Environment;
use dropdown::{CompletionCandidate, DropdownMenu};
use frecency_store::FrecencyStore;
use highlight::TokenType;
use rustyline::completion::{Completer, Pair};
use rustyline::highlight::Highlighter;
use rustyline::hint::{Hinter, HistoryHinter};
use rustyline::validate::{ValidationContext, ValidationResult, Validator};
use rustyline::{Context, Helper};
use spec::SpecRegistry;
use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Completion kinds worth remembering.
///
/// Filenames are deliberately absent: they are unbounded and mostly one-off, and letting them
/// into the table would drown the command ranking it exists to serve.
const RANKED_KINDS: &[&str] = &["command", "builtin", "subcommand"];

pub struct OsloHelper {
    env: Arc<Mutex<Environment>>,
    history_hinter: HistoryHinter,
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
    /// `which` answers for the line being drawn, keyed by the line's hash.
    ///
    /// Only path-like names reach it — a bare name is answered by [`command_index`] — but
    /// `highlight` runs on every refresh, so even one `stat` per token per keystroke is worth
    /// spending a hash lookup to avoid.
    which_cache: Mutex<(u64, HashMap<String, bool>)>,
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
            history_hinter: HistoryHinter::new(),
            spec_registry: SpecRegistry::new(),
            frecency,
            menu: interactive,
            editor_multiline: true,
            which_cache: Mutex::new((0, HashMap::new())),
        }
    }

    /// Turn the Tab dropdown on or off.
    ///
    /// With it off, `complete` returns the whole candidate list instead of the one the user
    /// picked — the shape a test wants.
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

    fn to_pair(candidate: CompletionCandidate) -> Pair {
        Pair {
            display: candidate.display,
            replacement: candidate.replacement,
        }
    }

    fn record_accepted(&self, candidate: &CompletionCandidate) {
        if candidate
            .kind
            .as_deref()
            .is_some_and(|k| RANKED_KINDS.contains(&k))
        {
            self.frecency.record(&candidate.display);
        }
    }
}

impl Completer for OsloHelper {
    type Candidate = Pair;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        _ctx: &Context<'_>,
    ) -> rustyline::Result<(usize, Vec<Pair>)> {
        let (start, candidates) = self.candidates(line, pos);

        if candidates.is_empty() {
            return Ok((start, Vec::new()));
        }
        if candidates.len() == 1 || !self.menu {
            if let [only] = candidates.as_slice() {
                // Unambiguous: rustyline inserts it without asking, which is an acceptance.
                self.record_accepted(only);
            }
            return Ok((start, candidates.into_iter().map(Self::to_pair).collect()));
        }

        let prompt_str = prompt::render_default_left_prompt(0);
        let indent_cols =
            dropdown::visible_len(&prompt_str) + dropdown::visible_len(&line[..start]);

        match DropdownMenu::select_interactive(candidates, indent_cols) {
            Some(selected) => {
                self.record_accepted(&selected);
                Ok((start, vec![Self::to_pair(selected)]))
            }
            None => Ok((start, Vec::new())),
        }
    }
}

impl Hinter for OsloHelper {
    type Hint = String;

    fn hint(&self, line: &str, pos: usize, ctx: &Context<'_>) -> Option<Self::Hint> {
        if line.is_empty() || pos < line.len() {
            return None;
        }

        // History wins: a line the user has actually run is a better guess than any name we could
        // rank, and it is what makes the suggestion feel like a memory rather than a directory
        // listing.
        if let Some(h) = self.history_hinter.hint(line, pos, ctx) {
            return Some(h);
        }

        self.command_hint(line, pos)
    }
}

impl Highlighter for OsloHelper {
    fn highlight<'l>(&self, line: &'l str, _pos: usize) -> Cow<'l, str> {
        if line.is_empty() {
            return Cow::Borrowed(line);
        }

        let path = self
            .env
            .lock()
            .unwrap()
            .get_var("PATH")
            .unwrap_or_default()
            .to_string();

        let mut out = String::with_capacity(line.len() * 2);
        for (tok, kind) in highlight::tokenize_for_highlight(line) {
            match kind {
                TokenType::Command => {
                    let valid = self.command_is_runnable(&tok, &path, line);
                    let colour = if valid { "1;32" } else { "1;31" };
                    out.push_str(&format!("\x1b[{}m{}\x1b[0m", colour, tok));
                }
                TokenType::Flag => out.push_str(&format!("\x1b[36m{}\x1b[0m", tok)),
                TokenType::String => out.push_str(&format!("\x1b[33m{}\x1b[0m", tok)),
                TokenType::Variable => out.push_str(&format!("\x1b[35m{}\x1b[0m", tok)),
                TokenType::Operator => out.push_str(&format!("\x1b[1;37m{}\x1b[0m", tok)),
                TokenType::Plain => out.push_str(&tok),
            }
        }

        Cow::Owned(out)
    }

    fn highlight_hint<'h>(&self, hint: &'h str) -> Cow<'h, str> {
        // Render fish-style ghost suggestion text in dim gray
        Cow::Owned(format!("\x1b[90m{}\x1b[0m", hint))
    }
}

impl OsloHelper {
    /// Whether a command token names something the shell could run, memoised per line.
    fn command_is_runnable(&self, name: &str, path: &str, line: &str) -> bool {
        let key = line_key(line);
        let mut cache = self.which_cache.lock().unwrap();
        if cache.0 != key {
            *cache = (key, HashMap::new());
        }
        if let Some(&known) = cache.1.get(name) {
            return known;
        }
        drop(cache);

        let answer = {
            let env = self.env.lock().unwrap();
            highlight::command_resolves(name, path, |n| {
                env.is_builtin(n) || env.get_alias(n).is_some() || env.get_function(n).is_some()
            })
        };

        let mut cache = self.which_cache.lock().unwrap();
        if cache.0 == key {
            cache.1.insert(name.to_string(), answer);
        }
        answer
    }
}

impl Validator for OsloHelper {
    fn validate(&self, ctx: &mut ValidationContext<'_>) -> rustyline::Result<ValidationResult> {
        let input = ctx.input();
        match syntax::classify(input) {
            InputStatus::Incomplete if self.editor_multiline => Ok(ValidationResult::Incomplete),
            InputStatus::Complete => {
                // Only when the editor is the thing assembling the command: otherwise `input` is
                // one line of a program the caller is still building, and the caller records the
                // finished buffer. Recording in both places would count every single-line command
                // twice and every multi-line command once, which is worse than counting neither.
                if self.editor_multiline {
                    self.record_command_use(input);
                }
                Ok(ValidationResult::Valid(None))
            }
            // A syntax error is not something another line can repair, and bash reports it from
            // the executor rather than the editor. Accept the line so the parser produces the
            // same diagnostic — and the same `$?` — it would for a script.
            _ => Ok(ValidationResult::Valid(None)),
        }
    }
}

impl Helper for OsloHelper {}

/// A cheap identity for the line being highlighted. Collisions only cost a stale colour.
fn line_key(line: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    line.hash(&mut hasher);
    hasher.finish()
}

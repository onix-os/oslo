//! The interactive line editor: completion, hints, colouring and multi-line input.
//!
//! [`OsloHelper`] is the rustyline `Helper`. The trait implementations here are deliberately thin
//! — each one delegates to a module that can be called directly from a test, because none of this
//! behaviour was reachable without a pty and that is precisely why it was all wrong.

pub mod command_index;
pub mod completion;
pub mod dropdown;
pub mod frecency_store;
pub mod highlight;
mod hinting;
pub mod keys;
pub mod marks;
pub mod prompt;
pub mod query;
pub mod row;
pub mod settings;
pub mod spec;
pub mod syntax;
pub mod theme;
pub mod vi;
pub mod words;

#[cfg(test)]
mod tests;

pub use command_index::invalidate as invalidate_command_cache;
pub use syntax::{DEFAULT_PS2, InputStatus};
pub use words::{Quote, Word, current_word, extract_current_word};

use crate::env::Environment;
use dropdown::{CompletionCandidate, DropdownMenu};
use frecency_store::FrecencyStore;
use rustyline::completion::{Completer, Pair};
use rustyline::highlight::Highlighter;
use rustyline::hint::{Hinter, HistoryHinter};
use rustyline::validate::{ValidationContext, ValidationResult, Validator};
use rustyline::{Context, Helper};
use spec::SpecRegistry;
use std::borrow::Cow;
use std::collections::HashSet;
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
    /// The right prompt to draw, and how many cells the left prompt took.
    ///
    /// Set once per prompt cycle by the REPL and drawn by `highlight`, which is the only seam
    /// where a cursor move is free — see [`prompt::right_prompt_escape`].
    right_prompt: Mutex<Option<(String, usize)>>,
    /// The language and status this prompt was drawn for, so a repaint rebuilds the same one.
    prompt_context: Mutex<(String, i32)>,
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
            right_prompt: Mutex::new(None),
            prompt_context: Mutex::new(("sh".to_string(), 0)),
        }
    }

    /// Turn the Tab dropdown on or off.
    ///
    /// With it off, `complete` returns the whole candidate list instead of the one the user
    /// picked — the shape a test wants.
    /// Give the helper the right prompt for this line, and the left prompt's width.
    pub fn set_right_prompt(&self, right: Option<String>, left_width: usize) {
        if let Ok(mut slot) = self.right_prompt.lock() {
            *slot = right.map(|text| (text, left_width));
        }
    }

    pub fn set_menu(&mut self, enabled: bool) {
        self.menu = enabled;
    }

    /// Which language this prompt is reading, and the status it was drawn with.
    ///
    /// Both are set by the REPL each time round, so a repaint rebuilds the same prompt rather than
    /// guessing at one.
    pub fn set_prompt_context(&self, language: &str, last_status: i32) {
        if let Ok(mut slot) = self.prompt_context.lock() {
            *slot = (language.to_string(), last_status);
        }
    }

    fn language(&self) -> String {
        self.prompt_context
            .lock()
            .map(|c| c.0.clone())
            .unwrap_or_else(|_| "sh".to_string())
    }

    fn last_status(&self) -> i32 {
        self.prompt_context.lock().map(|c| c.1).unwrap_or(0)
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

        let prompt_str = prompt::render_default_left_prompt(0, "sh");
        let indent_cols =
            dropdown::visible_len(&prompt_str) + dropdown::visible_len(&line[..start]);

        // What the user has typed of this word, so the dropdown can show which part of each
        // candidate is already theirs.
        let typed = &line[start..pos];
        match DropdownMenu::select_interactive(candidates, indent_cols, typed) {
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

        // The order is `oslo.suggest.sources`, defaulting to fish's: history, then completions,
        // then paths. Each answers for a position the others cannot see, and an empty list turns
        // suggestions off entirely.
        for source in settings::current().suggest.sources {
            let found = match source {
                settings::Source::History => self.history_hinter.hint(line, pos, ctx),
                settings::Source::Completion => self.command_hint(line, pos),
                settings::Source::Path => self.path_hint(line, pos),
            };
            if found.is_some() {
                return found;
            }
        }

        None
    }
}

impl Highlighter for OsloHelper {
    /// The prompt, re-rendered for whichever language the prompt is *now* reading.
    ///
    /// **This is what makes an in-place language switch stick.** The editor keeps the prompt
    /// string it was handed when the line started and writes that same string on every redraw —
    /// so a repaint that changed the language was reverted by the next keystroke, a completion,
    /// or anything else that refreshed the row. Rendering it here instead means every redraw is
    /// already correct and there is nothing to fight.
    ///
    /// Only the built-in prompt is rebuilt. A prompt from a Lua config is that config's business
    /// and is passed through untouched.
    fn highlight_prompt<'b, 's: 'b, 'p: 'b>(
        &'s self,
        prompt: &'p str,
        _default: bool,
    ) -> Cow<'b, str> {
        let Some(language) = prompt::language() else {
            return Cow::Borrowed(prompt);
        };
        let rebuilt = prompt::render_default_left_prompt(self.last_status(), &language);
        // Only when it really is the built-in prompt: same width means same layout, and the
        // editor's arithmetic is measured off the string it was given.
        if prompt::printed_width(&rebuilt) == prompt::printed_width(prompt) {
            Cow::Owned(rebuilt)
        } else {
            Cow::Borrowed(prompt)
        }
    }

    fn highlight<'l>(&self, line: &'l str, _pos: usize) -> Cow<'l, str> {
        // An empty line still gets the right prompt. Returning `Cow::Borrowed(line)` here is what
        // made it appear only after the first keystroke: rustyline draws the prompt, calls this
        // with `""`, and got nothing back — so the right prompt existed but was invisible until
        // you typed. There is no syntax to paint, but there is still a line to decorate.
        if line.is_empty() {
            return Cow::Owned(self.right_prompt_only(line));
        }

        let (path, builtins, functions) = {
            let env = self.env.lock().unwrap();
            let path = env.get_var("PATH").unwrap_or_default().to_string();
            // Snapshotted rather than queried per word: the closures below are called once per
            // command word, and each would otherwise take the environment lock again while this
            // one is still held.
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
            // A line long enough to make the syscalls add up is one nobody is reading the
            // colours of. See `highlight::MAX_PATH_CHECKS`.
            check_paths: line.len() <= 512,
        };
        // `OSC 133;B` goes first, so it lands between the prompt and the typed text — which is
        // where it means anything. rustyline measures the *raw* line and never this, so the mark
        // costs nothing in the cursor arithmetic. See `marks::input_start`.
        let mut painted = marks::input_start();
        painted.push_str(&highlight::paint(line, &ctx));

        // The right prompt rides here rather than in the prompt string: `compute_layout` measures
        // the raw line and never this, so a cursor move costs nothing in rustyline's arithmetic.
        if let Ok(slot) = self.right_prompt.lock()
            && let Some((right, left_width)) = slot.as_ref()
        {
            let used = left_width + prompt::printed_width(line);
            painted.push_str(&prompt::right_prompt_escape(
                right,
                used,
                dropdown::terminal_cols(),
            ));
            // Recorded so the vi-mode handler can draw this row again when the mode changes.
            // rustyline will not repaint a prompt and cannot be asked to, so oslo keeps enough to
            // do it itself. See `prompt::repaint`.
            prompt::note_row(&self.language(), self.last_status(), *left_width);
        }
        Cow::Owned(painted)
    }

    fn highlight_hint<'h>(&self, hint: &'h str) -> Cow<'h, str> {
        // The ghost suggestion, in whatever the theme calls `autosuggestion`.
        let theme = theme::current();
        Cow::Owned(theme.syntax.autosuggestion.paint(hint, theme::depth()))
    }
}

impl OsloHelper {
    /// The right prompt on its own, for a line with no syntax to paint.
    fn right_prompt_only(&self, line: &str) -> String {
        // The input mark belongs on an empty line too — that is the line you are about to type on.
        let mark = marks::input_start();
        let Ok(slot) = self.right_prompt.lock() else {
            return format!("{mark}{line}");
        };
        let Some((right, left_width)) = slot.as_ref() else {
            return format!("{mark}{line}");
        };
        let used = left_width + prompt::printed_width(line);
        format!(
            "{mark}{line}{}",
            prompt::right_prompt_escape(right, used, dropdown::terminal_cols())
        )
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

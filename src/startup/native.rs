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

use oslo::interactive::edit::session::{Assist, Bound, KeyHook};
use oslo::interactive::term::Key;
use oslo::interactive::{OsloHelper, abbr, dropdown, editor, marks, settings};

/// What the shell plugs into an editing session.
pub struct ShellAssist<'a> {
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
    /// The key that switches language — `crate::startup::mode::TOGGLE_KEY`, unless the config
    /// unbound it with `oslo.keys["shift-tab"] = "none"`.
    toggle: Option<String>,
}

impl<'a> ShellAssist<'a> {
    pub fn new(
        history: Vec<String>,
        helper: Option<&'a OsloHelper>,
        prompt_cols: usize,
        toggle: Option<String>,
    ) -> ShellAssist<'a> {
        ShellAssist {
            helper,
            prompt_cols,
            history,
            back: 0,
            composing: None,
            toggle,
        }
    }

    /// Start a fresh line: the walk position resets, or Up would resume where the last line left
    /// off and appear to skip entries.
    pub fn begin(&mut self) {
        self.back = 0;
        self.composing = None;
    }
}

/// oslo's name for a key, in the spelling `oslo.keys` uses.
///
/// `None` for anything a config could not name — an ordinary character, above all, since asking
/// Lua about every letter typed would put a hash lookup on the hot path for nothing.
fn key_name(key: Key) -> Option<String> {
    Some(match key {
        Key::Ctrl(c) => format!("ctrl-{c}"),
        Key::Alt(c) if !c.is_control() => format!("alt-{c}"),
        Key::ToggleScope => "tab".to_string(),
        Key::BackTab => "shift-tab".to_string(),
        Key::Up => "up".to_string(),
        Key::Down => "down".to_string(),
        Key::Left => "left".to_string(),
        Key::Right => "right".to_string(),
        Key::Home => "home".to_string(),
        Key::End => "end".to_string(),
        Key::PageUp => "pageup".to_string(),
        Key::PageDown => "pagedown".to_string(),
        // The chords `term` folded into shared names. A config that bound `ctrl-a` means the
        // chord, so the name has to come back out even though the key arrived as `Home`.
        Key::Clear => "ctrl-u".to_string(),
        _ => return None,
    })
}

/// oslo's name for a key, for the `key` hook — which sees *every* key.
///
/// A superset of [`key_name`], and separate from it on purpose. That one answers only for keys a
/// config could bind, because it feeds `oslo.keys` and a name there that cannot be bound would be
/// a lie. The hook has no such contract: it reports what was pressed, so Enter and Esc and an
/// ordinary letter all need a name.
///
/// The `char` half is `Some` only for an ordinary character, which is what lets a handler tell
/// `k.name == "char"` apart from a chord without comparing against a list.
///
/// **The names are the decoded ones.** `term` folds Ctrl-A into [`Key::Home`] before anything here
/// runs, so that key is reported as `"home"` — the same name `oslo.keys` uses for it, which is the
/// property worth keeping. A hook cannot recover the chord, and neither can a binding.
fn hook_key_name(key: Key) -> (String, Option<char>) {
    match key {
        Key::Char(c) => ("char".to_string(), Some(c)),
        Key::Ctrl(c) => (format!("ctrl-{c}"), None),
        Key::Alt(c) if !c.is_control() => (format!("alt-{c}"), None),
        Key::Alt(_) => ("alt-backspace".to_string(), None),
        Key::Accept => ("enter".to_string(), None),
        Key::Cancel => ("esc".to_string(), None),
        Key::Abort => ("ctrl-c".to_string(), None),
        Key::Backspace => ("backspace".to_string(), None),
        Key::Delete => ("delete".to_string(), None),
        Key::ToggleScope => ("tab".to_string(), None),
        Key::BackTab => ("shift-tab".to_string(), None),
        Key::Clear => ("ctrl-u".to_string(), None),
        Key::Up => ("up".to_string(), None),
        Key::Down => ("down".to_string(), None),
        Key::Left => ("left".to_string(), None),
        Key::Right => ("right".to_string(), None),
        Key::Home => ("home".to_string(), None),
        Key::End => ("end".to_string(), None),
        Key::PageUp => ("pageup".to_string(), None),
        Key::PageDown => ("pagedown".to_string(), None),
        Key::Ignored => ("ignored".to_string(), None),
    }
}

/// A character index into the byte index Lua counts in.
///
/// The editor's cursor is a position among *characters*; `line.cursor` is documented as bytes so
/// that `line.text:sub(1, line.cursor)` works, and Lua's own string functions count bytes. The two
/// agree for ASCII and diverge on the first accented letter, which is why this is not skipped.
fn byte_cursor(line: &str, cursor: usize) -> usize {
    line.chars().take(cursor).map(char::len_utf8).sum()
}

/// The byte index a handler answered with, back into a character index for the buffer.
fn char_cursor(line: &str, at: usize) -> usize {
    line.char_indices().take_while(|(i, _)| *i < at).count()
}

/// A handler's answer as the editor wants it: text, a character cursor, and whether to run it.
///
/// The cursor a handler gives is a byte offset, because that is what it computed with; the buffer
/// counts characters. Defaulting to the end is what "just set the line" means, and it is by far
/// the common case.
fn placed(answer: editor::Answer) -> (String, usize, bool) {
    let end = answer.text.chars().count();
    let cursor = answer
        .cursor
        .map(|at| char_cursor(&answer.text, at))
        .unwrap_or(end)
        .min(end);
    (answer.text, cursor, answer.submit)
}

/// Open the full-screen history finder.
///
/// `None` means it could not open at all — no terminal, no store, nothing remembered — and the
/// caller should carry on as though it had never been asked. That is a different answer from
/// `Some(Cancelled)`, which means the user looked and declined, and where carrying on would
/// scroll their line away as if Esc had done something.
fn open_finder(seed: &str) -> Option<oslo::interactive::finder::Outcome> {
    let settings = settings::current();
    if !settings.finder.enabled {
        return None;
    }
    let track = oslo::track::store()?;
    // Only this language's commands. The editor's history holds both, and offering a Lua line at a
    // shell prompt produces something that cannot run — the same crossing the ghost suggestion and
    // the arrow keys are already filtered for.
    let language = oslo::interactive::prompt::language().unwrap_or_else(|| "sh".to_string());
    let commands: Vec<_> = track
        .commands(settings.finder.limit)
        .into_iter()
        .filter(|command| command.mode == language)
        .collect();
    if commands.is_empty() {
        return None;
    }
    let cwd = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    oslo::interactive::finder::open(&commands, &cwd, now, settings.completion.fuzzy, seed)
}

impl Assist for ShellAssist<'_> {
    fn highlight(&mut self, line: &str) -> String {
        let Some(helper) = self.helper else {
            return line.to_string();
        };
        if line.is_empty() {
            return String::new();
        }
        // `OSC 133;B` first, so it lands between the prompt and the typed text, which is where it
        // means anything. It prints nothing, so it costs no cells.
        let mut painted = marks::input_start();
        painted.push_str(&helper.paint(line));
        painted
    }

    /// The ghost suggestion, from `oslo.suggest.sources` in order.
    ///
    /// Only at the end of the line: a suggestion is text that *continues* what you have typed, and
    /// appending it after a cursor sitting mid-line would be a claim about the wrong position.
    fn hint(&mut self, line: &str, cursor: usize) -> Option<String> {
        let text = self.hint_text(line, cursor)?;
        Some(self.helper?.paint_hint(&text))
    }

    /// The suggestion as plain text, which is what accepting it inserts.
    fn hint_text(&mut self, line: &str, cursor: usize) -> Option<String> {
        let helper = self.helper?;
        if cursor < line.chars().count() {
            return None;
        }
        helper.suggest(line, line.len())
    }

    /// Tab. Runs the whole interaction — the dropdown draws itself and takes its own keys — and
    /// answers with the line it produced.
    fn complete(&mut self, line: &str, cursor: usize, _back: bool) -> Option<(String, usize)> {
        let helper = self.helper?;
        // The dropdown works in bytes; the editor's cursor is in characters.
        let pos: usize = line.chars().take(cursor).map(char::len_utf8).sum();
        let (start, candidates) = helper.complete_word(line, pos);
        if candidates.is_empty() {
            return None;
        }

        let chosen = if candidates.len() == 1 {
            // Already recorded by `complete_word`, which is where "one candidate is an
            // acceptance" belongs so that every caller agrees.
            candidates.into_iter().next()?
        } else {
            let indent = self.prompt_cols + dropdown::visible_len(&line[..start]);
            let chosen =
                dropdown::DropdownMenu::select_interactive(candidates, indent, &line[start..pos])?;
            helper.record_accepted(&chosen);
            chosen
        };

        let mut out = String::with_capacity(line.len() + chosen.replacement.len());
        out.push_str(&line[..start]);
        out.push_str(&chosen.replacement);
        let at = out.chars().count();
        out.push_str(&line[pos..]);
        Some((out, at))
    }

    /// What the config bound `key` to.
    ///
    /// The order is the order of specificity: an `oslo.keys` entry is the most explicit statement
    /// a config makes, then the suggestion-accept keys, then oslo's own defaults. A default that
    /// could shadow a config entry would make the entry look ignored.
    fn binding(&mut self, key: Key) -> Option<Bound> {
        let name = key_name(key)?;
        let settings = settings::current();

        if let Some((_, action)) = settings.keys.iter().find(|(bound, _)| *bound == name) {
            return match oslo::interactive::keys::action(action) {
                Some(oslo::interactive::keys::Action::ToggleLanguage) => {
                    Some(Bound::ToggleLanguage)
                }
                Some(oslo::interactive::keys::Action::ClearScreen) => Some(Bound::ClearScreen),
                Some(oslo::interactive::keys::Action::HistorySearchBackward) => {
                    Some(Bound::SearchHistory)
                }
                Some(oslo::interactive::keys::Action::AcceptSuggestion) => Some(Bound::AcceptHint),
                Some(oslo::interactive::keys::Action::AcceptSuggestionWord) => {
                    Some(Bound::AcceptHintWord)
                }
                Some(oslo::interactive::keys::Action::Interrupt) => Some(Bound::Interrupt),
                Some(oslo::interactive::keys::Action::Complete) => Some(Bound::Complete),
                Some(oslo::interactive::keys::Action::LuaHandler) => Some(Bound::Lua(name)),
                // Unbound on purpose. Answering `None` here rather than with a do-nothing `Bound`
                // is what makes it reach the *defaults* below and cancel them too — which is the
                // whole point, since Shift-Tab is bound before any config has run.
                Some(oslo::interactive::keys::Action::Nothing) => return None,
                // An action name oslo does not know was already reported when the config was
                // read; doing nothing here is better than doing something arbitrary.
                None => None,
            };
        }

        if settings.suggest.accept.as_deref() == Some(name.as_str()) {
            return Some(Bound::AcceptHint);
        }
        if settings.suggest.accept_word.as_deref() == Some(name.as_str()) {
            return Some(Bound::AcceptHintWord);
        }

        // oslo's own default, for a key the ordinary keymap does not already answer. Reached only
        // when the config said nothing about this key: `oslo.keys["shift-tab"] = "none"` returns
        // above and so cancels it.
        (Some(name.as_str()) == self.toggle.as_deref()).then_some(Bound::ToggleLanguage)
    }

    fn key_name(&mut self, key: Key) -> Option<String> {
        let name = key_name(key)?;
        // **Asked of the handler registry, not of `oslo.keys` in the settings.** A binding whose
        // value is a *function* never reaches the settings — those hold only the name-to-action
        // strings — so filtering on them meant every Lua binding was silently never consulted.
        //
        // A hash lookup per named key is cheap, and `key_name` has already ruled out every
        // ordinary character, which is what keeps this off the path of simply typing.
        editor::handler(&name).is_some().then_some(name)
    }

    fn lua_key(&mut self, name: &str, line: &str, cursor: usize) -> Option<(String, usize, bool)> {
        let handler = editor::handler(name)?;
        let table = editor::line_table(line, byte_cursor(line, cursor));
        let answer = match oslo::lua::engine::call_here(&handler, vec![table]) {
            Ok(values) => values.into_iter().next().unwrap_or_default(),
            // Reported rather than swallowed: a binding that silently does nothing is
            // indistinguishable from one that was never installed.
            Err(e) => {
                eprintln!("oslo: keys['{name}']: {e}");
                return None;
            }
        };
        let answer = editor::answer_from(&answer)?;
        Some(placed(answer))
    }

    fn watches_keys(&mut self) -> bool {
        oslo::lua::engine::key_hook_watched()
    }

    fn key_hook(&mut self, key: Key, line: &str, cursor: usize) -> Option<KeyHook> {
        let (name, pressed) = hook_key_name(key);
        let table = editor::key_table(&name, pressed, line, byte_cursor(line, cursor));
        match editor::key_outcome_from(&oslo::lua::engine::key_hook_here(vec![table])?) {
            editor::KeyOutcome::Pass => None,
            editor::KeyOutcome::Swallow => Some(KeyHook::Swallow),
            editor::KeyOutcome::Line(answer) => {
                let (text, cursor, submit) = placed(answer);
                Some(KeyHook::Line {
                    text,
                    cursor,
                    submit,
                })
            }
        }
    }

    fn abbreviation(&mut self, line: &str, cursor: usize) -> Option<(String, usize)> {
        // The dropdown and `abbr` both work in bytes; the editor's cursor is in characters.
        let at: usize = line.chars().take(cursor).map(char::len_utf8).sum();
        let (mut text, expanded_to) = abbr::expand(line, at)?;
        // The space that triggered this, supplied here because this consumed the keystroke.
        text.insert(expanded_to, ' ');
        let cursor = text[..expanded_to + 1].chars().count();
        Some((text, cursor))
    }

    fn search_history(&mut self, line: &str) -> Option<String> {
        // Chosen, but **not run**: you may want to edit it first, which is the contract every
        // other recall in the shell has.
        match open_finder(line)? {
            oslo::interactive::finder::Outcome::Chosen { line, .. } => Some(line),
            oslo::interactive::finder::Outcome::Cancelled => None,
        }
    }

    fn history_prev(&mut self, line: &str) -> Option<String> {
        // Up opens the finder when the config asked for that, which is oslo's default: a
        // full-screen fuzzy search over what you have actually run.
        let settings = settings::current();
        if self.back == 0 && settings.finder.enabled && settings.finder.key == "up" {
            // Whatever is already on the line seeds the search: pressing Up after typing `ls`
            // means "the `ls` I ran before".
            match open_finder(line) {
                Some(oslo::interactive::finder::Outcome::Chosen { line, .. }) => return Some(line),
                // Looked and declined: leave the line exactly as it was rather than falling
                // through to a walk, which would scroll it away as if Esc had done something.
                Some(oslo::interactive::finder::Outcome::Cancelled) => return None,
                // Could not open — no terminal, or nothing remembered yet. Walk the history the
                // ordinary way, which is what Up meant before the finder existed.
                None => {}
            }
        }
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
            entries.iter().map(|e| e.to_string()).collect(),
            None,
            0,
            Some("shift-tab".to_string()),
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

    /// **Every key has a hook name.** A key the hook cannot name is a key it cannot be told about,
    /// and the whole point of the hook is that it sees all of them — so this asserts the ones
    /// `key_name` deliberately leaves out, which is where a missing arm would actually hide.
    #[test]
    fn the_key_hook_can_name_the_keys_a_binding_cannot() {
        for (key, name) in [
            (Key::Accept, "enter"),
            (Key::Cancel, "esc"),
            (Key::Abort, "ctrl-c"),
            (Key::Backspace, "backspace"),
            (Key::Delete, "delete"),
        ] {
            assert_eq!(hook_key_name(key).0, name);
            assert!(
                key_name(key).is_none(),
                "{name} is not bindable, which is why the hook needs its own table"
            );
        }
        // An ordinary character is the one case with a `char`, and they all share one name so a
        // handler can ask "was this typing?" without listing the alphabet.
        assert_eq!(
            hook_key_name(Key::Char('ß')),
            ("char".to_string(), Some('ß'))
        );
        assert_eq!(hook_key_name(Key::Ctrl('k')).0, "ctrl-k");
        assert_eq!(hook_key_name(Key::Ctrl('k')).1, None);
    }

    /// The cursor crosses the boundary between the editor, which counts characters, and Lua, which
    /// counts bytes — so `line.text:sub(1, line.cursor)` is the text before the cursor even when
    /// the line is not ASCII. Handing over the character index put the split in the wrong place.
    #[test]
    fn the_cursor_a_handler_sees_is_in_bytes() {
        let line = "größe x";
        // Six characters in, which is after the space: eight bytes, since ö and ß are two each.
        assert_eq!(byte_cursor(line, 6), 8);
        assert_eq!(&line[..byte_cursor(line, 6)], "größe ");
        // And back again, so a handler's answer lands where it asked.
        assert_eq!(char_cursor(line, 8), 6);
        for at in 0..=line.chars().count() {
            assert_eq!(char_cursor(line, byte_cursor(line, at)), at, "round trip");
        }
    }
}

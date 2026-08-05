//! The line-editing state machine, and the loop that drives it.
//!
//! Split so the interesting half is testable: [`Session::apply`] takes one key and mutates the
//! line, and knows nothing about terminals. The loop underneath only reads keys and writes bytes.
//!
//! # What the shell plugs in
//!
//! Everything oslo-specific — highlighting, ghost hints, the completion dropdown, history — comes
//! through [`Assist`]. That is deliberate: those are the parts that already exist and work, and
//! the point of this module is to stop rustyline owning the *layout*, not to rewrite completion.

use super::buffer::{Buffer, Case};
use super::keymap::{Action, action};
use super::{layout, screen};
use crate::interactive::dropdown::terminal_cols;
pub use crate::interactive::term::Key;
use crate::interactive::term::{Keys, Restore, Screen};
use std::io::Write;

mod assist;
pub use assist::{Assist, NoAssist};

/// What a `key` hook asked the editor to do with the keystroke it just saw.
///
/// The third possibility — carry on as normal — is the `None` the hook answers with, so it needs
/// no variant here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyHook {
    /// Consume the keystroke: the editor never sees it, and the line is untouched.
    Swallow,
    /// Put this line in place instead, and run it if `submit`.
    Line {
        text: String,
        cursor: usize,
        submit: bool,
    },
}

/// A binding the config asked for, which the session performs instead of its default.
///
/// Named for the effect rather than the key, because the same effect can be reached from a chord,
/// a config entry or a default — and the loop performing it should not care which.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Bound {
    /// Switch the prompt between shell and Lua.
    ToggleLanguage,
    ClearScreen,
    SearchHistory,
    /// Take the whole ghost suggestion.
    AcceptHint,
    /// Take one word of it.
    AcceptHintWord,
    Interrupt,
    Complete,
    /// A Lua function, by the key's name.
    Lua(String),
}

/// What a keypress did to the session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Keep editing. `redraw` is false for a key that changed nothing, so an unbound chord does
    /// not repaint the row.
    Continue { redraw: bool },
    /// Enter.
    Accept,
    /// Ctrl-C.
    Interrupted,
    /// Ctrl-D on an empty line.
    Eof,
    /// Ctrl-L: the screen should be cleared before the next draw.
    ClearScreen,
    /// Shift-Tab: the prompt should switch between shell and Lua. Handled by the read loop, which
    /// is the only thing that knows what a language *is*.
    ToggleLanguage,
}

/// The line being edited, and where in history it came from.
#[derive(Debug, Default)]
pub struct Session {
    pub buffer: Buffer,
    /// Vi mode, when `oslo.vi.enabled` asked for it.
    ///
    /// `None` is emacs, and then nothing vi-shaped is consulted at all. With it on, insert mode
    /// still passes every key through to the ordinary keymap — a vi user at a shell still expects
    /// `C-w` and `C-a` to work, because those are the shell's keys and not vi's.
    pub vi: Option<super::vi::Vi>,
}

impl Session {
    pub fn new(text: &str, cursor: usize) -> Session {
        let mut buffer = Buffer::new();
        buffer.set(text, cursor);
        Session {
            buffer,
            vi: crate::interactive::vi::enabled().then(super::vi::Vi::default),
        }
    }

    /// The mode a prompt should show, when there is one.
    ///
    /// **Read, not guessed.** `crate::interactive::vi::after_key` exists to infer this by watching
    /// keystrokes go past, because rustyline owned the mode and would not say — so the indicator
    /// was always one key behind. Here it is simply the field.
    pub fn mode(&self) -> Option<super::vi::Mode> {
        self.vi.as_ref().map(|vi| vi.mode)
    }

    /// Carry out a binding the config asked for.
    fn perform(&mut self, bound: Bound, assist: &mut dyn Assist) -> Step {
        let changed = |yes: bool| Step::Continue { redraw: yes };
        match bound {
            Bound::ToggleLanguage => Step::ToggleLanguage,
            Bound::ClearScreen => Step::ClearScreen,
            Bound::Interrupt => Step::Interrupted,
            Bound::Complete => {
                match assist.complete(&self.buffer.text(), self.buffer.cursor(), false) {
                    Some((line, cursor)) => {
                        self.buffer.set(&line, cursor);
                        changed(true)
                    }
                    None => changed(false),
                }
            }
            Bound::SearchHistory => match assist.search_history(&self.buffer.text()) {
                Some(line) => {
                    let end = line.chars().count();
                    self.buffer.set(&line, end);
                    changed(true)
                }
                None => changed(false),
            },
            Bound::AcceptHint => changed(self.take_hint(true, assist)),
            Bound::AcceptHintWord => changed(self.take_hint(false, assist)),
            Bound::Lua(name) => {
                match assist.lua_key(&name, &self.buffer.text(), self.buffer.cursor()) {
                    Some((line, cursor, submit)) => {
                        self.buffer.set(&line, cursor);
                        // `submit = true` is zsh's `bindkey -s '…\n'`: the key runs the line
                        // rather than only typing it.
                        if submit { Step::Accept } else { changed(true) }
                    }
                    None => changed(false),
                }
            }
        }
    }

    /// Take the ghost suggestion into the line — all of it, or one word.
    ///
    /// The suggestion is *what would be drawn now*, asked for again rather than remembered from
    /// the last frame — a remembered one can be stale by exactly the keystroke that accepted it.
    ///
    /// `false` when there was nothing to take, so a key that also means something else can fall
    /// through to that meaning.
    fn take_hint(&mut self, whole: bool, assist: &mut dyn Assist) -> bool {
        let line = self.buffer.text();
        let Some(hint) = assist.hint_text(&line, self.buffer.cursor()) else {
            return false;
        };
        let take = if whole { hint } else { first_word(&hint) };
        if take.is_empty() {
            return false;
        }
        self.buffer.move_end();
        self.buffer.insert_str(&take);
        true
    }

    /// Apply one key.
    pub fn apply(&mut self, key: Key, assist: &mut dyn Assist) -> Step {
        let changed = |yes: bool| Step::Continue { redraw: yes };

        // **The `key` hook sees the keystroke before anything else, ordinary characters included.**
        //
        // First, because a hook that cannot see a key before its binding runs cannot implement the
        // thing it exists for — deciding what a key *means*. That is also why it can answer with a
        // whole line: an observer would not need one.
        //
        // Gated on `watches_keys` so a session with no handler attached pays one atomic load per
        // keystroke rather than building the line to hand over. It is the only method on `Assist`
        // that runs for every key including the ones nobody bound, so it is the only one where
        // that distinction is worth drawing.
        if assist.watches_keys()
            && let Some(hook) = assist.key_hook(key, &self.buffer.text(), self.buffer.cursor())
        {
            return match hook {
                // Nothing happened and nothing is redrawn — the key is simply gone.
                KeyHook::Swallow => changed(false),
                KeyHook::Line {
                    text,
                    cursor,
                    submit,
                } => {
                    self.buffer.set(&text, cursor);
                    if submit { Step::Accept } else { changed(true) }
                }
            };
        }

        // **A key the config bound wins over everything, vi included.**
        //
        // Before vi rather than after, and that ordering is load-bearing: vi mode is on by
        // default, and it reads `Alt(x)` as Esc-then-`x` so that leaving insert mode at speed
        // works. That would make every `oslo.keys["alt-…"]` binding unreachable for most users —
        // an explicit binding has to beat a heuristic about what someone probably meant.
        if let Some(bound) = assist.binding(key) {
            return self.perform(bound, assist);
        }

        // **Right at the end of the line takes the ghost suggestion**, and moves the cursor
        // everywhere else. One key doing both jobs is fish's `forward-char`, and it is the key
        // people reach for — Tab opens the dropdown when there is a choice to make, Right says
        // "yes, that one". `hint_text` answers `None` unless the cursor is already at the end, so
        // Right mid-line is unaffected and needs no check here.
        //
        // **Not in vi's normal mode.** There `l` and Right are a motion, and a motion has to be a
        // motion — `d<Right>` deletes a character, and a Right that inserted text instead would
        // make the operator do something no vi user asked for. Insert and replace are where text
        // gets added, so that is where a key can add some.
        //
        // Above vi rather than in the keymap because vi sees the key first and would move the
        // cursor before the keymap ever ran.
        if key == Key::Right
            && self.mode() != Some(super::vi::Mode::Normal)
            && self.take_hint(true, assist)
        {
            return changed(true);
        }

        // Vi gets next refusal. `Passthrough` means the key is not vi's business — insert mode,
        // or Enter in any mode — and falls through to the ordinary keymap below.
        if let Some(vi) = self.vi.as_mut()
            && let super::vi::Outcome::Handled { redraw } = vi.apply(key, &mut self.buffer)
        {
            return Step::Continue { redraw };
        }

        match action(key) {
            // A space may end an abbreviation. Tried before the space is inserted, because the
            // expansion supplies its own — the two are one step.
            Action::Insert(' ') => {
                if let Some((line, cursor)) =
                    assist.abbreviation(&self.buffer.text(), self.buffer.cursor())
                {
                    self.buffer.set(&line, cursor);
                    return changed(true);
                }
                self.buffer.insert(' ');
                changed(true)
            }
            Action::Insert(c) => {
                self.buffer.insert(c);
                changed(true)
            }

            Action::Left => {
                self.buffer.move_left();
                changed(true)
            }
            Action::Right => {
                self.buffer.move_right();
                changed(true)
            }
            Action::WordLeft => {
                self.buffer.move_word_left();
                changed(true)
            }
            Action::WordRight => {
                self.buffer.move_word_right();
                changed(true)
            }
            Action::Home => {
                self.buffer.move_home();
                changed(true)
            }
            Action::End => {
                self.buffer.move_end();
                changed(true)
            }

            Action::Backspace => changed(self.buffer.backspace()),
            // **Ctrl-D is two keys in one.** On a line with text it deletes forward; on an empty
            // one it is end of input, which is how every shell has ended a session since v7.
            // Only here is the line known to be empty, which is why the keymap cannot decide it.
            Action::Delete => {
                if self.buffer.is_empty() {
                    Step::Eof
                } else {
                    changed(self.buffer.delete())
                }
            }
            Action::KillToEnd => changed(self.buffer.kill_to_end()),
            Action::KillToStart => changed(self.buffer.kill_to_start()),
            Action::KillWordLeft => changed(self.buffer.kill_word_left()),
            Action::KillWordRight => changed(self.buffer.kill_word_right()),
            Action::KillSpaceWordLeft => changed(self.buffer.kill_space_word_left()),
            Action::Yank => changed(self.buffer.yank()),
            Action::Transpose => changed(self.buffer.transpose()),
            Action::Upper => changed(self.buffer.case_word(Case::Upper)),
            Action::Lower => changed(self.buffer.case_word(Case::Lower)),
            Action::Capitalise => changed(self.buffer.case_word(Case::Title)),

            Action::Complete | Action::CompleteBack => {
                let back = matches!(action(key), Action::CompleteBack);
                match assist.complete(&self.buffer.text(), self.buffer.cursor(), back) {
                    Some((line, cursor)) => {
                        self.buffer.set(&line, cursor);
                        changed(true)
                    }
                    None => changed(false),
                }
            }
            Action::HistoryPrev => match assist.history_prev(&self.buffer.text()) {
                Some(line) => {
                    let end = line.chars().count();
                    self.buffer.set(&line, end);
                    changed(true)
                }
                None => changed(false),
            },
            Action::HistoryNext => match assist.history_next() {
                Some(line) => {
                    let end = line.chars().count();
                    self.buffer.set(&line, end);
                    changed(true)
                }
                None => changed(false),
            },
            Action::SearchHistory => match assist.search_history(&self.buffer.text()) {
                Some(line) => {
                    let end = line.chars().count();
                    self.buffer.set(&line, end);
                    changed(true)
                }
                None => changed(false),
            },

            Action::Accept => Step::Accept,
            Action::Abort => Step::Interrupted,
            Action::Eof => Step::Eof,
            Action::Redraw => Step::ClearScreen,
            // Esc on its own does nothing in emacs mode. It is not "cancel": a shell prompt has
            // nothing to cancel *to*, and abandoning the line is Ctrl-C's job.
            Action::Escape | Action::None => changed(false),
        }
    }
}

/// How reading a line ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Line(String),
    /// The language toggle was pressed. The read loop switches and reopens with the same text, so
    /// the line and the cursor survive the switch — which is the whole point of a toggle that
    /// works mid-line.
    ToggleLanguage {
        text: String,
        cursor: usize,
    },
    /// Ctrl-C: this line is abandoned, the shell carries on.
    Interrupted,
    /// Ctrl-D on an empty line, or the input ended.
    Eof,
}

/// Read one line.
///
/// `initial` is text to start with and where to put the cursor in it — how a reopened line, a
/// history recall or a finder choice comes back.
pub fn read_line(
    prompt: &str,
    right: &str,
    initial: (&str, usize),
    assist: &mut dyn Assist,
) -> Outcome {
    let Some(raw) = Restore::enter(Screen::Line) else {
        // No terminal: the line comes off stdin with no editing, which is what a piped script
        // needs and what `read` already does elsewhere.
        //
        return read_plain(prompt);
    };
    let mut session = Session::new(initial.0, initial.1);
    let mut keys = Keys::on(raw.fd());
    let mut at_row = 0usize;
    let mut out = std::io::stderr();
    let mut drawn = false;
    let mut idle = false;

    loop {
        // The cursor shape says which mode you are in, as fish's vi mode does. `observe` publishes
        // the mode for the prompt to read and answers with an escape only when it actually
        // changed, so an unchanged mode costs nothing per keystroke.
        if let Some(mode) = session.mode()
            && let Some(shape) = crate::interactive::vi::observe(
                mode,
                &crate::interactive::settings::current().vi.cursors,
            )
        {
            let _ = out.write_all(shape.as_bytes());
        }

        let placed = draw(prompt, right, &session, assist, true);
        let _ = out.write_all(screen::redraw(at_row, &placed.text, into_at(&placed)).as_bytes());
        let _ = out.flush();
        at_row = placed.cursor_row;

        // **`post-prompt`: the prompt is now on the screen.** Once per line, not once per frame —
        // the loop redraws on every keystroke, and a hook firing there would be an `on-key` with a
        // worse name. This is the first moment anything could be seen, which is what "after the
        // prompt is displayed" has to mean.
        if !drawn {
            drawn = true;
            crate::lua::engine::fire_at_here(crate::lua::api::hooks::at::POST_PROMPT, &[]);
        }

        let Some(key) = next_key(&mut keys, &mut idle) else {
            return Outcome::Eof;
        };
        match session.apply(key, assist) {
            Step::Continue { .. } => {}
            Step::ToggleLanguage => {
                let placed = draw(prompt, right, &session, assist, true);
                // Back to the top of the block and erase it: the caller redraws from the same row
                // with the other language's prompt, so leaving this one would double it.
                let _ = out.write_all(screen::redraw(at_row, "", into_at(&placed)).as_bytes());
                let _ = out.flush();
                return Outcome::ToggleLanguage {
                    text: session.buffer.text(),
                    cursor: session.buffer.cursor(),
                };
            }
            Step::ClearScreen => {
                // Home the cursor and clear, then fall through to a normal redraw from row 0.
                let _ = out.write_all(b"\x1b[H\x1b[2J");
                at_row = 0;
            }
            Step::Accept => {
                // The next line starts in insert, so the shape must go back — otherwise a line
                // accepted from normal mode leaves a block cursor over the one you type next.
                crate::interactive::vi::reset();
                if session.vi.is_some() {
                    let shape = crate::interactive::settings::current()
                        .vi
                        .cursors
                        .for_mode(crate::interactive::vi::Mode::Insert);
                    let _ = out.write_all(shape.escape().as_bytes());
                }
                let placed = draw(prompt, right, &session, assist, false);
                let _ = out
                    .write_all(screen::redraw(at_row, &placed.text, into_at(&placed)).as_bytes());
                let _ = out.write_all(screen::finish(placed.cursor_row, placed.rows).as_bytes());
                let _ = out.flush();
                return Outcome::Line(session.buffer.text());
            }
            // The abandoned line stays on screen — it is what you just typed, and erasing it
            // takes away the thing you might want to look at or copy. Only the cursor moves,
            // down past the block so the next prompt starts on a clean row.
            step @ (Step::Interrupted | Step::Eof) => {
                let placed = draw(prompt, right, &session, assist, false);
                let _ = out.write_all(screen::finish(placed.cursor_row, placed.rows).as_bytes());
                let _ = out.flush();
                return match step {
                    Step::Eof => Outcome::Eof,
                    _ => Outcome::Interrupted,
                };
            }
        }
    }
}

/// The next key, firing `on-idle-timeout` if the prompt sits untouched long enough.
///
/// **The blocking read is the default and stays the default.** A timed read is only asked for when
/// `oslo.misc.idle_timeout` is set *and* something is attached to the hook — otherwise the editor
/// would wake up on a timer for the rest of the session to ask a question nobody is listening for.
///
/// `reported` is what stops it firing over and over: idleness is a state you enter once, not a
/// tick. It resets the moment a key arrives, so walking away twice reports twice.
fn next_key(keys: &mut Keys, reported: &mut bool) -> Option<Key> {
    let seconds = crate::interactive::settings::current().misc.idle_timeout;
    if seconds == 0 || !crate::lua::api::hooks::watched(crate::lua::api::hooks::at::IDLE_TIMEOUT) {
        return keys.read();
    }
    let ms = seconds.saturating_mul(1000).min(i32::MAX as u64) as i32;
    loop {
        match keys.read_within(ms) {
            crate::interactive::term::Pressed::Key(key) => {
                *reported = false;
                return Some(key);
            }
            crate::interactive::term::Pressed::Timeout => {
                if !*reported {
                    *reported = true;
                    crate::lua::engine::fire_at_here(
                        crate::lua::api::hooks::at::IDLE_TIMEOUT,
                        &[("seconds", &seconds.to_string())],
                    );
                }
            }
            crate::interactive::term::Pressed::Ended => return None,
        }
    }
}

/// Build the frame for the current state.
/// Lay the line out. `ghost` is whether the suggestion is drawn with it.
///
/// **Off for the last frame of a line.** The ghost is a proposal, not text you typed — so once the
/// line is finished it has to go, or the transcript shows a command that was never run. Typing
/// `cat ~/` with `lis/` suggested and pressing Enter left `cat ~/lis/` on screen above the output
/// of `cat ~/`, which is a scrollback that lies about what happened.
fn draw(
    prompt: &str,
    right: &str,
    session: &Session,
    assist: &mut dyn Assist,
    ghost: bool,
) -> layout::Placed {
    let plain = session.buffer.text();
    let painted = assist.highlight(&plain);
    let hint = if ghost {
        assist
            .hint(&plain, session.buffer.cursor())
            .unwrap_or_default()
    } else {
        String::new()
    };
    layout::place(&layout::Row {
        prompt,
        text: &painted,
        plain: &plain,
        cursor: session.buffer.cursor(),
        hint: &hint,
        right,
        // Read every frame rather than cached, so a resized terminal lays out correctly on the
        // next keystroke without a `SIGWINCH` handler to get wrong.
        cols: terminal_cols(),
    })
}

fn into_at(placed: &layout::Placed) -> screen::At {
    screen::At {
        rows: placed.rows,
        cursor_row: placed.cursor_row,
        cursor_col: placed.cursor_col,
    }
}

/// A line from stdin, for when there is no terminal to edit on.
fn read_plain(prompt: &str) -> Outcome {
    // The prompt is written only for a terminal that cannot be edited on — `TERM=dumb`, a serial
    // console, a `screen` session someone has told to be simple. Down an ordinary pipe it is not
    // written at all: the shell is being driven by a script, and a prompt interleaved with the
    // output would be noise in the middle of the data.
    if std::env::var("TERM").as_deref() == Ok("dumb") {
        print!("{prompt}");
        let _ = std::io::Write::flush(&mut std::io::stdout());
    }
    let mut line = String::new();
    match std::io::BufRead::read_line(&mut std::io::stdin().lock(), &mut line) {
        Ok(0) | Err(_) => Outcome::Eof,
        Ok(_) => {
            while line.ends_with('\n') || line.ends_with('\r') {
                line.pop();
            }
            Outcome::Line(line)
        }
    }
}

#[cfg(test)]
#[path = "session/tests.rs"]
mod tests;

/// The first word of a suggestion, with the whitespace that follows it.
///
/// Accepting "one word" of `--example foo` should leave the cursor after the space, ready for the
/// next word — stopping before it would make the second press insert a leading space.
fn first_word(hint: &str) -> String {
    let trimmed = hint.trim_start();
    let lead = hint.len() - trimmed.len();
    let end = trimmed
        .find(char::is_whitespace)
        .map(|at| {
            let rest = &trimmed[at..];
            at + rest.len() - rest.trim_start().len()
        })
        .unwrap_or(trimmed.len());
    hint[..lead + end].to_string()
}

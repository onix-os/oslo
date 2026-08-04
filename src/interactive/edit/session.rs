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
use crate::interactive::term::{Key, Keys, Restore, Screen};
use std::io::Write;

/// What the shell supplies to an editing session.
///
/// Every method has a default that does nothing, so a test — or an early integration — can
/// implement none of them and still get a working editor.
pub trait Assist {
    /// The line as it should be drawn. Must print the same *characters*: the layout measures the
    /// plain text and draws this, so adding or removing anything but escapes moves the cursor.
    fn highlight(&mut self, line: &str) -> String {
        line.to_string()
    }

    /// Ghost text shown after the cursor, already styled.
    fn hint(&mut self, _line: &str, _cursor: usize) -> Option<String> {
        None
    }

    /// Run completion, answering the line and cursor it produced.
    ///
    /// The whole interaction belongs to the implementation — oslo's dropdown draws itself and
    /// takes its own keys — because a menu is a different mode, not a keystroke.
    fn complete(&mut self, _line: &str, _cursor: usize, _back: bool) -> Option<(String, usize)> {
        None
    }

    /// The previous history entry, given what is on the line now.
    fn history_prev(&mut self, _line: &str) -> Option<String> {
        None
    }

    fn history_next(&mut self) -> Option<String> {
        None
    }

    /// Ctrl-R. Answers a whole line to put in place, or `None` to leave things alone.
    fn search_history(&mut self, _line: &str) -> Option<String> {
        None
    }
}

/// An `Assist` that does nothing, for tests and for a shell that has not wired one yet.
#[derive(Debug, Default)]
pub struct NoAssist;
impl Assist for NoAssist {}

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
}

/// The line being edited, and where in history it came from.
#[derive(Debug, Default)]
pub struct Session {
    pub buffer: Buffer,
}

impl Session {
    pub fn new(text: &str, cursor: usize) -> Session {
        let mut buffer = Buffer::new();
        buffer.set(text, cursor);
        Session { buffer }
    }

    /// Apply one key.
    pub fn apply(&mut self, key: Key, assist: &mut dyn Assist) -> Step {
        let changed = |yes: bool| Step::Continue { redraw: yes };
        match action(key) {
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
        return read_plain();
    };
    let mut session = Session::new(initial.0, initial.1);
    let mut keys = Keys::on(raw.fd());
    let mut at_row = 0usize;
    let mut out = std::io::stderr();

    loop {
        let placed = draw(prompt, right, &session, assist);
        let _ = out.write_all(screen::redraw(at_row, &placed.text, into_at(&placed)).as_bytes());
        let _ = out.flush();
        at_row = placed.cursor_row;

        let Some(key) = keys.read() else {
            return Outcome::Eof;
        };
        match session.apply(key, assist) {
            Step::Continue { .. } => {}
            Step::ClearScreen => {
                // Home the cursor and clear, then fall through to a normal redraw from row 0.
                let _ = out.write_all(b"\x1b[H\x1b[2J");
                at_row = 0;
            }
            Step::Accept => {
                let placed = draw(prompt, right, &session, assist);
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
                let placed = draw(prompt, right, &session, assist);
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

/// Build the frame for the current state.
fn draw(prompt: &str, right: &str, session: &Session, assist: &mut dyn Assist) -> layout::Placed {
    let plain = session.buffer.text();
    let painted = assist.highlight(&plain);
    let hint = assist
        .hint(&plain, session.buffer.cursor())
        .unwrap_or_default();
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
fn read_plain() -> Outcome {
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

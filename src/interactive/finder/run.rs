//! Opening the finder: raw mode, the key loop, and putting the terminal back.
//!
//! # The one rule
//!
//! **Whatever happens, the terminal is restored.** The alternate screen, the cursor, the original
//! termios — all three are undone on every path out, including the ones nobody plans for. A finder
//! that leaves the screen in its own mode does not merely look wrong; it leaves a shell you cannot
//! type into, and the only way out is to close the window. So the restore is a guard that runs on
//! drop rather than a line at the end of the loop, and the loop below cannot forget it.

use super::Scope;
use super::rank::{Ranked, rank};
use super::render::{Frame, frame, visible_rows};
use crate::interactive::matching::Fuzzy;
use crate::interactive::term::{Key, Keys, Restore};
use crate::track::history::Command;
use std::io::{self, Write};

/// What the finder was left with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// A line to put back on the prompt, and the language it was typed in.
    Chosen { line: String, mode: String },
    /// Esc, or Ctrl-C: the prompt comes back exactly as it was.
    Cancelled,
}

/// Open the finder over `commands` and run it until it is dismissed.
///
/// `None` when there is no terminal to draw on or nothing to show — a finder that opened onto an
/// empty screen would be a keystroke that appears to hang.
pub fn open(commands: &[Command], cwd: &str, now: i64, fuzzy: Fuzzy) -> Option<Outcome> {
    if commands.is_empty() {
        return None;
    }
    // Raw mode and the alternate screen, both undone whichever way this returns. The prompt
    // underneath is untouched and comes back exactly as it was, which is the whole reason a
    // full-screen finder is affordable here.
    let restore = Restore::enter(true)?;

    let mut stdout = io::stdout();
    let mut state = State::new(commands, cwd, fuzzy);
    let mut keys = Keys::on(restore.fd());

    loop {
        let (cols, rows) = terminal_size();
        state.fit(rows);
        let painted = frame(&Frame {
            matches: &state.matches,
            selected: state.selected,
            offset: state.offset,
            query: &state.query,
            scope: state.scope,
            total: state.total(),
            cols,
            rows,
            now,
        });
        let _ = stdout.write_all(painted.as_bytes());
        let _ = stdout.flush();

        let Some(pressed) = keys.read() else {
            return Some(Outcome::Cancelled);
        };

        match pressed {
            Key::Cancel => return Some(Outcome::Cancelled),
            Key::Accept => {
                return Some(match state.matches.get(state.selected) {
                    Some(chosen) => Outcome::Chosen {
                        line: chosen.command.line.clone(),
                        mode: chosen.command.mode.clone(),
                    },
                    // Enter on an empty result list dismisses rather than choosing nothing:
                    // there is no line to put back, and clearing what the prompt already had
                    // would lose work.
                    None => Outcome::Cancelled,
                });
            }
            Key::Up => state.up(),
            Key::Down => state.down(),
            Key::PageUp => state.page_up(),
            Key::PageDown => state.page_down(),
            Key::ToggleScope => state.toggle_scope(),
            Key::Backspace => {
                state.query.pop();
                state.refilter();
            }
            Key::Clear => {
                state.query.clear();
                state.refilter();
            }
            Key::Char(c) => {
                state.query.push(c);
                state.refilter();
            }
            // Keys the shared reader knows but the finder has no use for: a full-screen list
            // has no cursor to move within a line.
            Key::Ignored
            | Key::Delete
            | Key::Left
            | Key::Right
            | Key::Home
            | Key::End
            | Key::BackTab => {}
        }
    }
}

/// The finder's state between keystrokes.
struct State<'a> {
    commands: &'a [Command],
    cwd: String,
    fuzzy: Fuzzy,
    query: String,
    scope: Scope,
    matches: Vec<Ranked>,
    /// Index into `matches`.
    selected: usize,
    /// First visible row, so a list longer than the screen scrolls under a fixed window.
    offset: usize,
    /// How many rows the list currently has. Set from the terminal each frame, because it can be
    /// resized while the finder is open.
    window: usize,
}

impl<'a> State<'a> {
    fn new(commands: &'a [Command], cwd: &str, fuzzy: Fuzzy) -> State<'a> {
        let matches = rank(commands, "", cwd, fuzzy);
        State {
            commands,
            cwd: cwd.to_string(),
            fuzzy,
            query: String::new(),
            scope: Scope::Global,
            matches,
            // The list grows upward from the search bar, so the row *nearest* the bar is the one
            // selected when it opens: the best match, under the cursor, one keystroke away.
            selected: 0,
            offset: 0,
            window: 1,
        }
    }

    fn refilter(&mut self) {
        self.matches = rank(self.commands, &self.query, &self.cwd, self.fuzzy);
        if self.scope == Scope::Local {
            self.matches.retain(|row| row.command.dir == self.cwd);
        }
        // Back to the top: the old selection referred to a list that no longer exists, and
        // keeping the index would land the cursor on an unrelated command.
        self.selected = 0;
        self.offset = 0;
    }

    fn toggle_scope(&mut self) {
        self.scope = match self.scope {
            Scope::Global => Scope::Local,
            Scope::Local => Scope::Global,
        };
        self.refilter();
    }

    fn total(&self) -> usize {
        match self.scope {
            Scope::Global => self.commands.len(),
            Scope::Local => self
                .commands
                .iter()
                .filter(|command| command.dir == self.cwd)
                .count(),
        }
    }

    /// Results are stored best-first but painted bottom-up. Moving visually upward therefore
    /// advances through the vector; moving down goes back toward index zero.
    fn up(&mut self) {
        self.move_by(1);
    }

    fn down(&mut self) {
        self.move_by(-1);
    }

    fn page_up(&mut self) {
        self.move_by(self.window as isize);
    }

    fn page_down(&mut self) {
        self.move_by(-(self.window as isize));
    }

    /// Move the selection, clamped, and bring it back into view.
    fn move_by(&mut self, delta: isize) {
        if self.matches.is_empty() {
            return;
        }
        let last = self.matches.len() - 1;
        let next = (self.selected as isize + delta).clamp(0, last as isize) as usize;
        self.selected = next;
        self.scroll_into_view();
    }

    /// Note this frame's window size and keep the selection visible in it.
    fn fit(&mut self, rows: usize) {
        self.window = visible_rows(rows);
        self.scroll_into_view();
    }

    fn scroll_into_view(&mut self) {
        if self.selected < self.offset {
            self.offset = self.selected;
        } else if self.selected >= self.offset + self.window {
            self.offset = self.selected + 1 - self.window;
        }
        // A window that grew past the end of the list would leave blank rows below the last match
        // and above the bar, which reads as the list having ended early.
        let max_offset = self.matches.len().saturating_sub(self.window);
        self.offset = self.offset.min(max_offset);
    }
}

/// The terminal's size, re-asked every frame because it can be resized while the finder is open.
fn terminal_size() -> (usize, usize) {
    (
        crate::interactive::dropdown::width::terminal_cols(),
        crate::interactive::dropdown::width::terminal_rows(),
    )
}

#[cfg(test)]
#[path = "run/tests.rs"]
mod tests;

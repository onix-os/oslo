//! Opening the finder: raw mode, the key loop, and putting the terminal back.
//!
//! # The one rule
//!
//! **Whatever happens, the terminal is restored.** The alternate screen, the cursor, the original
//! termios — all three are undone on every path out, including the ones nobody plans for. A finder
//! that leaves the screen in its own mode does not merely look wrong; it leaves a shell you cannot
//! type into, and the only way out is to close the window. So the restore is a guard that runs on
//! drop rather than a line at the end of the loop, and the loop below cannot forget it.

use super::rank::{Ranked, rank};
use super::render::{Frame, frame};
use crate::interactive::matching::Fuzzy;
use crate::track::history::Command;
use nix::sys::termios::{LocalFlags, SetArg, Termios, tcgetattr, tcsetattr};
use std::io::{self, Read, Write};

/// What the finder was left with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// A line to put back on the prompt, and the language it was typed in.
    Chosen { line: String, mode: String },
    /// Esc, or Ctrl-C: the prompt comes back exactly as it was.
    Cancelled,
}

/// Restores the terminal when it goes out of scope.
///
/// A guard rather than a call at the end, because the paths out of the loop are many — a chosen
/// line, a cancel, a read that fails, a panic in the renderer — and every one of them has to leave
/// a usable terminal behind. This is the only thing standing between a bug in the loop and a
/// window the user has to close.
struct Restore {
    stdin: io::Stdin,
    original: Termios,
}

impl Drop for Restore {
    fn drop(&mut self) {
        let mut stdout = io::stdout();
        // Leave the alternate screen and show the cursor again, then hand the terminal's modes
        // back. In that order: the escapes have to reach a terminal that is still in raw mode, or
        // the shell's own prompt is drawn over whatever is left of ours.
        let _ = stdout.write_all(b"\x1b[?25h\x1b[?1049l");
        let _ = stdout.flush();
        let _ = tcsetattr(&self.stdin, SetArg::TCSANOW, &self.original);
    }
}

/// Open the finder over `commands` and run it until it is dismissed.
///
/// `None` when there is no terminal to draw on or nothing to show — a finder that opened onto an
/// empty screen would be a keystroke that appears to hang.
pub fn open(
    commands: &[Command],
    cwd: &str,
    home: &str,
    now: i64,
    fuzzy: Fuzzy,
) -> Option<Outcome> {
    if commands.is_empty() || !io::IsTerminal::is_terminal(&io::stdin()) {
        return None;
    }

    let stdin = io::stdin();
    let original = tcgetattr(&stdin).ok()?;
    let mut raw = original.clone();
    raw.local_flags.remove(LocalFlags::ICANON);
    raw.local_flags.remove(LocalFlags::ECHO);
    tcsetattr(&stdin, SetArg::TCSANOW, &raw).ok()?;
    let _restore = Restore {
        stdin: io::stdin(),
        original,
    };

    let mut stdout = io::stdout();
    // Alternate screen, cursor hidden. The prompt underneath is untouched and comes back exactly
    // as it was — which is the whole reason a full-screen finder is affordable here.
    let _ = stdout.write_all(b"\x1b[?1049h\x1b[?25l");

    let mut state = State::new(commands, cwd, fuzzy);
    let mut keys = io::stdin();

    loop {
        let (cols, rows) = terminal_size();
        state.fit(rows);
        let painted = frame(&Frame {
            matches: &state.matches,
            selected: state.selected,
            offset: state.offset,
            query: &state.query,
            total: commands.len(),
            cols,
            rows,
            now,
            home,
        });
        let _ = stdout.write_all(painted.as_bytes());
        let _ = stdout.flush();

        let mut buf = [0u8; 8];
        let read = match keys.read(&mut buf) {
            Ok(0) | Err(_) => return Some(Outcome::Cancelled),
            Ok(n) => n,
        };

        match key(&buf[..read]) {
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
            Key::Up => state.move_by(-1),
            Key::Down => state.move_by(1),
            Key::PageUp => state.move_by(-(state.window as isize)),
            Key::PageDown => state.move_by(state.window as isize),
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
            Key::Ignored => {}
        }
    }
}

/// The finder's state between keystrokes.
struct State<'a> {
    commands: &'a [Command],
    cwd: String,
    fuzzy: Fuzzy,
    query: String,
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
        // Back to the top: the old selection referred to a list that no longer exists, and
        // keeping the index would land the cursor on an unrelated command.
        self.selected = 0;
        self.offset = 0;
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
        self.window = rows.saturating_sub(2).max(1);
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

/// What a keypress means here.
#[derive(Debug, PartialEq, Eq)]
enum Key {
    Char(char),
    Backspace,
    Up,
    Down,
    PageUp,
    PageDown,
    Accept,
    Cancel,
    Clear,
    Ignored,
}

/// Read one keypress out of the bytes a terminal delivered.
///
/// **Up and Down are inverted against the screen.** The list grows upward from the search bar, so
/// Down moves toward the bar — toward the best match — which is the direction the key points on
/// screen. Getting this backwards is the single most noticeable thing a finder can get wrong.
fn key(bytes: &[u8]) -> Key {
    match bytes {
        [] => Key::Ignored,
        // Esc alone. A lone `0x1b` with nothing after it is the key; anything after it is a
        // sequence, handled below.
        [0x1b] => Key::Cancel,
        [0x03] => Key::Cancel,
        [0x0d] | [0x0a] => Key::Accept,
        [0x7f] | [0x08] => Key::Backspace,
        // Ctrl-U, as it clears the line at a prompt.
        [0x15] => Key::Clear,
        // Ctrl-P / Ctrl-N, for the same reason every other list in the shell takes them.
        [0x10] => Key::Up,
        [0x0e] => Key::Down,
        [0x1b, b'[', rest @ ..] => match rest {
            [b'A', ..] => Key::Up,
            [b'B', ..] => Key::Down,
            [b'5', b'~', ..] => Key::PageUp,
            [b'6', b'~', ..] => Key::PageDown,
            _ => Key::Ignored,
        },
        [0x1b, b'O', rest @ ..] => match rest {
            [b'A', ..] => Key::Up,
            [b'B', ..] => Key::Down,
            _ => Key::Ignored,
        },
        // Anything printable is query text. Decoded as UTF-8 because a terminal delivers a
        // multibyte character in one read, and dropping it would make the finder unusable in any
        // language whose commands are not ASCII.
        bytes => match std::str::from_utf8(bytes) {
            Ok(text) => match text.chars().next() {
                Some(c) if !c.is_control() => Key::Char(c),
                _ => Key::Ignored,
            },
            Err(_) => Key::Ignored,
        },
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

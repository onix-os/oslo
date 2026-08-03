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
use crate::track::history::Command;
use nix::sys::termios::{LocalFlags, SetArg, Termios, tcgetattr, tcsetattr};
use std::io::{self, Write};

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
pub fn open(commands: &[Command], cwd: &str, now: i64, fuzzy: Fuzzy) -> Option<Outcome> {
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
    let mut keys = Keys::default();

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

        let Some(pressed) = keys.next() else {
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
    ToggleScope,
    Ignored,
}

/// What a byte sequence means, once a whole one has been collected.
///
/// Up and Down name screen directions, not vector directions. Results are stored best-first and
/// drawn bottom-up, so the loop advances the index for Up and reduces it for Down.
fn key(bytes: &[u8]) -> Key {
    match bytes {
        [] => Key::Ignored,
        // Esc alone. A lone `0x1b` with nothing after it is the key; anything after it is a
        // sequence, handled below.
        [0x1b] => Key::Cancel,
        [0x03] => Key::Cancel,
        [0x0d] | [0x0a] => Key::Accept,
        [0x7f] | [0x08] => Key::Backspace,
        [0x09] => Key::ToggleScope,
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
        [0x1b, ..] => Key::Ignored,
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

/// Reads whole keypresses, however the terminal chose to split them.
///
/// # Why this is not a `read` into a small buffer
///
/// It was, and the bug was visible immediately: a mouse report is
/// `ESC [ < 35 ; 56 ; 12 M` — thirteen bytes — and tmux, hexe and any terminal with mouse
/// reporting on send one for every movement. Read eight bytes at a time, the tail of each report
/// arrives as an ordinary read and every character of it is typed into the query. That is where
/// the `;5;;5;;6;66;6;5995` came from: not keys, but the second halves of escape sequences the
/// first read had cut in two.
///
/// So bytes are buffered and a **complete** sequence is taken from the front, or nothing is. An
/// unrecognised sequence is consumed whole and discarded rather than falling through to the text
/// path, which is what makes a mouse wheel do nothing instead of typing.
#[derive(Default)]
struct Keys {
    buf: Vec<u8>,
}

/// What the front of the buffer holds.
#[derive(Debug)]
enum Parsed {
    /// A key, and how many bytes it took.
    Took(usize, Key),
    /// A complete sequence that means nothing here. Consumed so it cannot be read as text.
    Discard(usize),
    /// A sequence that has not all arrived.
    Partial,
}

impl Keys {
    /// The next keypress, or `None` at end of input.
    fn next(&mut self) -> Option<Key> {
        loop {
            match parse(&self.buf) {
                Parsed::Took(used, key) => {
                    self.buf.drain(..used);
                    return Some(key);
                }
                Parsed::Discard(used) => {
                    self.buf.drain(..used);
                    continue;
                }
                Parsed::Partial => {
                    // A lone `ESC` that nothing follows is the Esc key. Everything else waits for
                    // the rest of itself. The pause is the standard one: long enough that a real
                    // sequence is never split, short enough that Esc feels immediate.
                    if self.buf == [0x1b] && !waiting(25) {
                        self.buf.clear();
                        return Some(Key::Cancel);
                    }
                    if !self.fill() {
                        return None;
                    }
                }
            }
        }
    }

    /// Read whatever is available. False at end of input.
    fn fill(&mut self) -> bool {
        let mut chunk = [0u8; 64];
        loop {
            // SAFETY: a slice this call owns, read from the terminal's own descriptor.
            let n = unsafe { nix::libc::read(0, chunk.as_mut_ptr().cast(), chunk.len()) };
            if n > 0 {
                self.buf.extend_from_slice(&chunk[..n as usize]);
                return true;
            }
            if n == 0 {
                return false;
            }
            // A window resize, most likely — not the user leaving.
            if io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
                return false;
            }
        }
    }
}

/// Whether another byte is already waiting, within `ms`.
fn waiting(ms: i32) -> bool {
    let mut fds = nix::libc::pollfd {
        fd: 0,
        events: nix::libc::POLLIN,
        revents: 0,
    };
    // SAFETY: one descriptor, its own length, and a timeout — nothing borrowed beyond the call.
    unsafe { nix::libc::poll(&mut fds, 1, ms) > 0 }
}

/// Take one key from the front of `buf`.
fn parse(buf: &[u8]) -> Parsed {
    let Some(&first) = buf.first() else {
        return Parsed::Partial;
    };
    if first != 0x1b {
        // A character, which may be several bytes. A truncated one waits rather than being read
        // as the replacement character.
        let len = utf8_len(first);
        if buf.len() < len {
            return Parsed::Partial;
        }
        return match std::str::from_utf8(&buf[..len]) {
            Ok(text) => Parsed::Took(len, key(text.as_bytes())),
            Err(_) => Parsed::Discard(len),
        };
    }
    match buf.get(1) {
        None => Parsed::Partial,
        // CSI: parameters, then intermediates, then one final byte that ends it. Everything a
        // terminal sends unprompted — mouse reports, bracketed paste markers, cursor position
        // replies, focus events — is one of these, which is why finding the final byte is the
        // whole job.
        Some(b'[') => {
            let mut at = 2;
            while at < buf.len() && (0x30..=0x3f).contains(&buf[at]) {
                at += 1;
            }
            while at < buf.len() && (0x20..=0x2f).contains(&buf[at]) {
                at += 1;
            }
            let Some(&final_byte) = buf.get(at) else {
                return Parsed::Partial;
            };
            if !(0x40..=0x7e).contains(&final_byte) {
                return Parsed::Discard(at + 1);
            }
            // The X10 mouse encoding is `ESC [ M` and *then* three raw bytes, which are not part
            // of the sequence by any rule — the only way to skip them is to know they are there.
            if final_byte == b'M' && at == 2 {
                return if buf.len() >= 6 {
                    Parsed::Discard(6)
                } else {
                    Parsed::Partial
                };
            }
            Parsed::Took(at + 1, key(&buf[..at + 1]))
        }
        // SS3: one final byte. This is how a terminal in application-cursor mode sends the arrows.
        Some(b'O') => match buf.get(2) {
            None => Parsed::Partial,
            Some(_) => Parsed::Took(3, key(&buf[..3])),
        },
        // OSC: runs to a BEL or a string terminator. A terminal answering a query oslo asked
        // earlier can land here.
        Some(b']') => {
            let mut at = 2;
            while at < buf.len() {
                if buf[at] == 0x07 {
                    return Parsed::Discard(at + 1);
                }
                if buf[at] == 0x1b && buf.get(at + 1) == Some(&b'\\') {
                    return Parsed::Discard(at + 2);
                }
                at += 1;
            }
            Parsed::Partial
        }
        // `ESC` then an ordinary character is Alt-that-character, which the finder does not bind.
        Some(_) => Parsed::Discard(2),
    }
}

/// How many bytes the character starting with `first` occupies.
fn utf8_len(first: u8) -> usize {
    match first {
        0x00..=0x7f => 1,
        0xc0..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf7 => 4,
        // A continuation byte with no lead: not a character start, so take it alone and let the
        // caller discard it.
        _ => 1,
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

//! Owning the terminal for a moment: raw mode, and reading whole keypresses out of it.
//!
//! Every full-screen thing oslo draws needs the same two pieces — put the terminal in raw mode and
//! be certain of putting it back, then read keys that a terminal may deliver in pieces. They live
//! here rather than in any one widget because there is now more than one: the history finder, and
//! everything in [`crate::interactive::ask`].
//!
//! # The reader is the interesting half
//!
//! A terminal does not deliver keypresses; it delivers bytes, and a single Up arrow is three of
//! them while a mouse report is thirteen. Read into a small buffer and the long ones get cut in
//! half — after which the tail arrives looking exactly like somebody typing. That is not a
//! hypothetical: it is where `;5;;5;;6;66;6;5995` came from in the finder's search box, once, with
//! tmux's mouse reporting on.
//!
//! So bytes are buffered and a **complete** sequence is taken from the front or nothing is, and
//! anything unrecognised is consumed whole rather than falling through to the text path.

use nix::sys::termios::{LocalFlags, SetArg, Termios, tcgetattr, tcsetattr};
use std::fs::File;
use std::io::{self, Write};
use std::os::fd::{AsRawFd, OwnedFd};

/// The descriptor to read keys from, and how it was found.
///
/// **Not always stdin.** `ls | ui choose` hands the *items* in on stdin, so the keyboard has to
/// come from somewhere else — and that somewhere is `/dev/tty`, the controlling terminal, which is
/// what every program in this shape opens. Reading keys from a pipe would block forever on input
/// that is never coming, which is exactly how the first version of this hung.
pub struct Tty {
    /// Kept alive for as long as the widget is running; `None` when stdin itself is the terminal.
    _owned: Option<OwnedFd>,
    fd: i32,
}

impl Tty {
    /// The terminal to talk to, or `None` when there is not one.
    pub fn open() -> Option<Tty> {
        if io::IsTerminal::is_terminal(&io::stdin()) {
            return Some(Tty {
                _owned: None,
                fd: libc_stdin(),
            });
        }
        // stdin is a pipe or a file, so the person is still at `/dev/tty` even though their
        // keystrokes are not arriving on fd 0.
        let file = File::options()
            .read(true)
            .write(true)
            .open("/dev/tty")
            .ok()?;
        let fd = file.as_raw_fd();
        let owned: OwnedFd = file.into();
        Some(Tty {
            fd,
            _owned: Some(owned),
        })
    }

    pub fn fd(&self) -> i32 {
        self.fd
    }
}

fn libc_stdin() -> i32 {
    nix::libc::STDIN_FILENO
}

/// Raw mode, and the promise of putting the terminal back.
///
/// A guard rather than a call at the end, because the paths out of the loop are many — a chosen
/// line, a cancel, a read that fails, a panic in the renderer — and every one of them has to leave
/// a usable terminal behind. This is the only thing standing between a bug in the loop and a
/// window the user has to close.
pub struct Restore {
    tty: Tty,
    original: Termios,
    /// Whether the alternate screen was entered, so leaving is not attempted when it was not.
    alternate: bool,
}

impl Restore {
    /// The descriptor keys should be read from while this guard is alive.
    pub fn fd(&self) -> i32 {
        self.tty.fd()
    }
}

impl Restore {
    /// Put the terminal into raw mode. `alternate` also takes the alternate screen and hides the
    /// cursor, which is what a full-screen widget wants and an inline one does not.
    ///
    /// `None` when there is no terminal, which every caller treats as "do not draw".
    pub fn enter(alternate: bool) -> Option<Restore> {
        let tty = Tty::open()?;
        // SAFETY: the descriptor is owned by `tty` and outlives every use of this borrow.
        let handle = unsafe { std::os::fd::BorrowedFd::borrow_raw(tty.fd()) };
        let original = tcgetattr(handle).ok()?;
        let mut raw = original.clone();
        raw.local_flags.remove(LocalFlags::ICANON);
        raw.local_flags.remove(LocalFlags::ECHO);
        tcsetattr(handle, SetArg::TCSANOW, &raw).ok()?;
        if alternate {
            let mut out = io::stdout();
            let _ = out.write_all(b"\x1b[?1049h\x1b[?25l");
            let _ = out.flush();
        }
        Some(Restore {
            tty,
            original,
            alternate,
        })
    }
}

impl Drop for Restore {
    fn drop(&mut self) {
        let mut stdout = io::stdout();
        // Show the cursor and leave the alternate screen, then hand the terminal's modes back. In
        // that order: the escapes have to reach a terminal that is still in raw mode, or the
        // shell's own prompt is drawn over whatever is left of ours.
        if self.alternate {
            let _ = stdout.write_all(b"\x1b[?25h\x1b[?1049l");
        }
        let _ = stdout.flush();
        // SAFETY: as above — the descriptor is `self.tty`'s and is still open here.
        let handle = unsafe { std::os::fd::BorrowedFd::borrow_raw(self.tty.fd()) };
        let _ = tcsetattr(handle, SetArg::TCSANOW, &self.original);
    }
}

/// What a keypress means.
#[derive(Debug, PartialEq, Eq)]
pub enum Key {
    Char(char),
    Backspace,
    /// Forward delete, which is not Backspace and must not be treated as it.
    Delete,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Accept,
    Cancel,
    /// Clear the line: Ctrl-U, as at a prompt.
    Clear,
    /// Tab. Named for what the finder does with it; other widgets use it to toggle a selection,
    /// which is the same "switch between two states" gesture.
    ToggleScope,
    /// Shift-Tab, which a terminal spells `ESC [ Z`.
    BackTab,
    Ignored,
}

/// What a byte sequence means, once a whole one has been collected.
///
/// Up and Down name screen directions, not vector directions. Results are stored best-first and
/// drawn bottom-up, so the loop advances the index for Up and reduces it for Down.
pub fn key(bytes: &[u8]) -> Key {
    match bytes {
        [] => Key::Ignored,
        // Esc alone. A lone `0x1b` with nothing after it is the key; anything after it is a
        // sequence, handled below.
        [0x1b] => Key::Cancel,
        [0x03] => Key::Cancel,
        [0x0d] | [0x0a] => Key::Accept,
        [0x7f] | [0x08] => Key::Backspace,
        [0x09] => Key::ToggleScope,
        // Ctrl-A / Ctrl-E, the line-start and line-end every readline-shaped thing binds.
        [0x01] => Key::Home,
        [0x05] => Key::End,
        // Ctrl-B / Ctrl-F, likewise.
        [0x02] => Key::Left,
        [0x06] => Key::Right,
        // Ctrl-D is a forward delete on a line that has text, and "I am done" on one that does
        // not. The widget decides which, because only it knows whether the line is empty.
        [0x04] => Key::Delete,
        // Ctrl-U, as it clears the line at a prompt.
        [0x15] => Key::Clear,
        // Ctrl-P / Ctrl-N, for the same reason every other list in the shell takes them.
        [0x10] => Key::Up,
        [0x0e] => Key::Down,
        [0x1b, b'[', rest @ ..] => match rest {
            [b'A', ..] => Key::Up,
            [b'B', ..] => Key::Down,
            [b'C', ..] => Key::Right,
            [b'D', ..] => Key::Left,
            [b'H', ..] => Key::Home,
            [b'F', ..] => Key::End,
            [b'Z', ..] => Key::BackTab,
            [b'1', b'~', ..] | [b'7', b'~', ..] => Key::Home,
            [b'3', b'~', ..] => Key::Delete,
            [b'4', b'~', ..] | [b'8', b'~', ..] => Key::End,
            [b'5', b'~', ..] => Key::PageUp,
            [b'6', b'~', ..] => Key::PageDown,
            _ => Key::Ignored,
        },
        [0x1b, b'O', rest @ ..] => match rest {
            [b'A', ..] => Key::Up,
            [b'B', ..] => Key::Down,
            [b'C', ..] => Key::Right,
            [b'D', ..] => Key::Left,
            [b'H', ..] => Key::Home,
            [b'F', ..] => Key::End,
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
pub struct Keys {
    buf: Vec<u8>,
    /// Where the keystrokes are. See [`Tty`] for why this is not always stdin.
    fd: i32,
}

impl Keys {
    /// Read keys from `fd`, which a caller gets from [`Restore::fd`].
    pub fn on(fd: i32) -> Keys {
        Keys {
            buf: Vec::new(),
            fd,
        }
    }
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
    /// Block until one whole keypress has arrived. `None` at end of input.
    ///
    /// Not `Iterator`: this blocks, and a `for key in keys` that waits on a person is a loop
    /// nobody reading it expects to stop.
    pub fn read(&mut self) -> Option<Key> {
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
                    if self.buf == [0x1b] && !waiting(self.fd, 25) {
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
            let n = unsafe { nix::libc::read(self.fd, chunk.as_mut_ptr().cast(), chunk.len()) };
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
fn waiting(fd: i32, ms: i32) -> bool {
    let mut fds = nix::libc::pollfd {
        fd,
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

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

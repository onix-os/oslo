//! The terminal side: what you are looking at while a tab is attached.
//!
//! It copies bytes and watches for one of them. Everything else — what `^C` means, whether the line
//! is being edited, which program owns the screen — belongs to the pty at the other end and to the
//! shell standing on it.
//!
//! # Raw, and stricter than the editor's raw
//!
//! In raw mode a keystroke stops being a keystroke and becomes a byte, which is exactly what makes
//! this invisible: `^C` arrives as `0x03`, is forwarded, and the *pty's* line discipline turns it
//! into `SIGINT` for the pty's foreground group. Nothing here has to know that `^C` means anything.
//!
//! oslo's line editor has its own raw mode and it is **not strict enough to reuse**. It clears
//! `ICANON`, `ECHO` and `ISIG` and leaves `IXON` and `OPOST` alone, which is right for an editor and
//! wrong here: `IXON` would let `^S` freeze this proxy instead of reaching the program inside, and
//! `OPOST` would translate newlines a second time on the way out of a pty that has already done it.

use super::{detach, store, wire};
use nix::poll::{PollFd, PollFlags, PollTimeout, poll};
use nix::sys::termios::{
    ControlFlags, InputFlags, LocalFlags, OutputFlags, SetArg, Termios, tcgetattr, tcsetattr,
};
use std::io::{self, Read, Write};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd};
use std::os::unix::net::UnixStream;

/// How an attached session ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Left {
    /// The detach key was pressed. The tab is still running.
    Detached,
    /// The tab ended — the shell inside exited, or the keeper went away.
    Ended,
}

/// Attach to `name` until the tab ends or the detach key is pressed.
///
/// The terminal is put in raw mode for the duration and put back however this returns, including
/// when it returns an error: a proxy that left a terminal raw would leave a shell that cannot be
/// typed into.
pub fn attach(
    mut socket: UnixStream,
    name: &str,
    detach: detach::Key,
    replay: u64,
) -> io::Result<Left> {
    // **The log is read from the file, not asked for over the socket**, which is what lets the two
    // backends share this: with a daemon in the way the bytes come through it, but the log is still
    // a file in the same directory, written by the same keeper.
    let paths = store::Paths::new(name);

    let stdin = io::stdin();
    let restore = Raw::take(stdin.as_fd())?;

    // **The screen belongs to one tab at a time.** Whatever is on it was written by somebody else —
    // the shell you pressed the key in, or the tab you just left — and replaying this tab's output
    // on top of it interleaves two sessions into one screen that belongs to neither.
    take_screen();
    // What the tab printed while nobody was watching, so an attach lands on the screen it left
    // rather than on a blank one.
    if replay > 0
        && let Ok(tail) = super::log::tail(&paths.log(), replay)
    {
        let _ = io::stdout().write_all(&tail);
        let _ = io::stdout().flush();
    }
    // The tab has no idea how big this terminal is until it is told, and it has just changed hands.
    let _ = tell_size(&mut socket);

    let outcome = pump(&mut socket, stdin.as_fd(), detach);
    // The screen the key was pressed on comes back untouched — its scrollback is the work of
    // whoever was there, and leaving a tab is not a reason to lose it.
    give_screen_back();
    drop(restore);
    outcome
}

/// Connect, allowing for a keeper that was made a moment ago.
///
/// **A tab exists before it listens.** `spawn` returns as soon as the fork has happened, and the
/// keeper binds its socket a few syscalls later — so a caller that made a tab and attached to it in
/// the same breath would fail against one that is perfectly healthy. Retrying briefly is the
/// difference; a tab that is genuinely not there fails just as surely, only a moment later.
pub fn connect(path: &std::path::Path, name: &str) -> io::Result<UnixStream> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        match UnixStream::connect(path) {
            Ok(socket) => return Ok(socket),
            Err(err) if std::time::Instant::now() >= deadline => {
                return Err(io::Error::other(format!("{name}: {err}")));
            }
            Err(_) => std::thread::sleep(std::time::Duration::from_millis(5)),
        }
    }
}

/// Copy until one side stops.
fn pump(socket: &mut UnixStream, stdin: BorrowedFd<'_>, detach: detach::Key) -> io::Result<Left> {
    let mut buffer = [0u8; wire::MAX];
    let mut out = io::stdout();
    loop {
        let (typed, from_pad) = {
            let mut fds = [
                PollFd::new(stdin, PollFlags::POLLIN),
                PollFd::new(socket.as_fd(), PollFlags::POLLIN),
            ];
            if poll(&mut fds, PollTimeout::NONE).map_err(errno)? == 0 {
                continue;
            }
            (ready(&fds[0]), ready(&fds[1]))
        };

        if from_pad {
            match socket.read(&mut buffer) {
                // The keeper has gone, which means the shell inside has.
                Ok(0) | Err(_) => return Ok(Left::Ended),
                Ok(n) => {
                    out.write_all(&buffer[..n])?;
                    out.flush()?;
                }
            }
        }

        if typed {
            match nix::unistd::read(stdin.as_raw_fd(), &mut buffer).map_err(errno) {
                Ok(0) | Err(_) => return Ok(Left::Ended),
                Ok(n) => {
                    // The one chord this side reads rather than forwards. Everything before it in
                    // the same read still goes, so a paste that happens to contain it is not lost.
                    if let Some(at) = detach.find(&buffer[..n]) {
                        if at > 0 {
                            socket.write_all(&wire::data(&buffer[..at]))?;
                        }
                        return Ok(Left::Detached);
                    }
                    socket.write_all(&wire::data(&buffer[..n]))?;
                }
            }
        }
    }
}

/// Take the screen for the tab, on the terminal's second one.
///
/// **The alternate screen rather than a clear.** Both give the tab a blank screen to itself, and
/// only this one gives the first screen *back*: the terminal keeps it, scrollback and all, and
/// redraws it untouched on the way out. Clearing instead — which is what tab-rs does, since it has
/// no shell of its own to return to — threw away the scrollback of the session the key was pressed
/// in, which is somebody else's work and not a tab's to discard.
///
/// **And not `ESC c` to tidy it, however tempting.** A full reset is what tab-rs sends, and on a
/// real terminal it takes the alternate screen down with it and empties the scrollback — undoing
/// the one thing this is for. What is left is the state a tab could otherwise inherit and draw
/// wrongly for: a scroll region somebody set, a hidden cursor, and the screen itself.
fn take_screen() {
    let mut out = io::stdout();
    let _ = out.write_all(b"\x1b[?1049h\x1b[r\x1b[?25h\x1b[H\x1b[2J");
    let _ = out.flush();
}

/// Give the first screen back, exactly as it was.
fn give_screen_back() {
    let mut out = io::stdout();
    let _ = out.write_all(b"\x1b[?1049l");
    let _ = out.flush();
}

/// Tell the tab how big this terminal is.
pub fn tell_size(socket: &mut UnixStream) -> io::Result<()> {
    let mut size = nix::libc::winsize {
        ws_row: 0,
        ws_col: 0,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // SAFETY: `size` outlives the call and TIOCGWINSZ writes exactly one `winsize` into it.
    let ok =
        unsafe { nix::libc::ioctl(io::stdin().as_raw_fd(), nix::libc::TIOCGWINSZ, &mut size) == 0 };
    if !ok || size.ws_row == 0 {
        return Ok(());
    }
    socket.write_all(&wire::resize(size.ws_row, size.ws_col))
}

/// The terminal's settings, put back when this is dropped.
pub struct Raw {
    fd: std::os::fd::OwnedFd,
    original: Termios,
}

impl Raw {
    /// Put the terminal into the proxy's raw mode, keeping what it was.
    pub fn take(fd: BorrowedFd<'_>) -> io::Result<Raw> {
        let original = tcgetattr(fd).map_err(errno)?;
        let mut raw = original.clone();

        // Nothing interprets anything: no line editing, no echo, no signals from keys, no flow
        // control, no newline translation in either direction.
        raw.local_flags
            .remove(LocalFlags::ICANON | LocalFlags::ECHO | LocalFlags::ISIG | LocalFlags::IEXTEN);
        raw.input_flags.remove(
            InputFlags::IXON
                | InputFlags::ICRNL
                | InputFlags::INLCR
                | InputFlags::IGNCR
                | InputFlags::BRKINT
                | InputFlags::ISTRIP,
        );
        raw.output_flags.remove(OutputFlags::OPOST);
        raw.control_flags |= ControlFlags::CS8;
        raw.control_chars[nix::libc::VMIN] = 1;
        raw.control_chars[nix::libc::VTIME] = 0;

        tcsetattr(fd, SetArg::TCSANOW, &raw).map_err(errno)?;
        Ok(Raw {
            fd: fd.try_clone_to_owned()?,
            original,
        })
    }
}

impl Drop for Raw {
    fn drop(&mut self) {
        let _ = tcsetattr(self.fd.as_fd(), SetArg::TCSANOW, &self.original);
    }
}

fn ready(fd: &PollFd<'_>) -> bool {
    fd.revents()
        .is_some_and(|events| events.intersects(PollFlags::POLLIN | PollFlags::POLLHUP))
}

fn errno(err: nix::errno::Errno) -> io::Error {
    io::Error::from_raw_os_error(err as i32)
}

#[cfg(test)]
mod tests;

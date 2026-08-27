//! Giving the terminal back when the shell dies without unwinding.
//!
//! [`super::Restore`] puts the terminal back in its `Drop`, which is right for every ordinary exit
//! and runs for none of the others. The release profile sets `panic = "abort"`, and an abort skips
//! destructors entirely — so a panic anywhere in the editor left the user at a terminal with
//! `ICANON`, `ECHO` and `ISIG` still cleared: nothing echoed, no line editing, and Ctrl-C dead.
//! The only way out was to blind-type `reset`.
//!
//! A panic *hook* runs before the abort, which is the one moment left. So the flags the editor
//! changed are stashed here when it takes the terminal, and the hook puts them back.
//!
//! # Why this is a copy of `Drop for Restore` rather than a call to it
//!
//! The hook cannot reach the guard: it is a local of the read loop, and by the time the hook runs
//! the stack it lives on is the one that is failing. What the hook needs is the handful of facts
//! required to undo the change, which is what `Saved` below holds — and those are copied out at the
//! moment the change is made, when they are known to be true.
//!
//! Kept deliberately small and allocation-free: a panic hook runs in a process that is already
//! going down, possibly out of memory, possibly inside the allocator.

use nix::sys::termios::{SetArg, Termios, tcsetattr};
use std::os::fd::RawFd;
use std::sync::{Mutex, OnceLock};

/// What has to be undone, copied out when the editor takes the terminal.
struct Saved {
    fd: RawFd,
    original: Termios,
    alternate: bool,
    bracketed_paste: bool,
    kitty_keyboard: bool,
    legacy_mouse: bool,
}

fn slot() -> &'static Mutex<Option<Saved>> {
    static SLOT: OnceLock<Mutex<Option<Saved>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

/// Note that the editor now holds the terminal, and how to give it back.
pub(super) fn remember(
    fd: RawFd,
    original: &Termios,
    alternate: bool,
    bracketed_paste: bool,
    kitty_keyboard: bool,
    legacy_mouse: bool,
) {
    install_hook();
    if let Ok(mut slot) = slot().lock() {
        *slot = Some(Saved {
            fd,
            original: original.clone(),
            alternate,
            bracketed_paste,
            kitty_keyboard,
            legacy_mouse,
        });
    }
}

/// The editor has given the terminal back the ordinary way; there is nothing left to rescue.
pub(super) fn forget() {
    if let Ok(mut slot) = slot().lock() {
        *slot = None;
    }
}

/// Descriptors dup2'd somewhere else, and the copies that put them back.
///
/// **Because a panic behind a redirect is a silent death.** `.env.lua` evaluation runs with stdout
/// and stderr pointing at an unlinked scratch file so its output can be shown tidily, and restores
/// them afterwards — after a `catch_unwind` that `panic = "abort"` never returns from. So a panic
/// anywhere in that evaluation wrote its message into a file nobody will ever read and killed an
/// interactive shell with a blank terminal.
///
/// Held as `(where it was moved from, the copy that restores it)`, restored before the terminal is,
/// so the message the default hook is about to print reaches a descriptor somebody can see.
fn redirected() -> &'static Mutex<Vec<(RawFd, RawFd)>> {
    static SLOT: OnceLock<Mutex<Vec<(RawFd, RawFd)>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(Vec::new()))
}

/// Note that `target` has been pointed elsewhere and `saved` is the copy that puts it back.
pub fn holding(target: RawFd, saved: RawFd) {
    install_hook();
    if let Ok(mut held) = redirected().lock() {
        held.push((target, saved));
    }
}

/// The descriptors have been put back the ordinary way.
pub fn released(target: RawFd) {
    if let Ok(mut held) = redirected().lock() {
        held.retain(|(at, _)| *at != target);
    }
}

/// Put the terminal back, if the editor is holding it.
///
/// Idempotent, and safe to call when nothing was taken. Writes to the descriptor directly rather
/// than through `stderr()` — a panic may be *inside* the standard-error lock, and taking it again
/// would deadlock a process that is trying to die.
fn give_it_back() {
    // **First**, so whatever the default hook prints next lands somewhere a person can see it
    // rather than in an unlinked scratch file. Taken rather than borrowed: this runs once.
    if let Ok(mut held) = redirected().lock() {
        for (target, saved) in held.drain(..) {
            let _ = nix::unistd::dup2(saved, target);
        }
    }
    let Ok(mut slot) = slot().lock() else {
        return;
    };
    let Some(saved) = slot.take() else {
        return;
    };

    let mut sequence: Vec<u8> = Vec::new();
    if saved.legacy_mouse {
        sequence.extend_from_slice(super::mouse::DISABLE);
    }
    if saved.kitty_keyboard {
        sequence.extend_from_slice(super::keyboard::POP.as_bytes());
    }
    if saved.bracketed_paste {
        sequence.extend_from_slice(super::BRACKETED_PASTE_DISABLE);
    }
    // The cursor comes back on either way; the alternate screen is left as well when it was entered.
    sequence.extend_from_slice(match saved.alternate {
        true => b"\x1b[?25h\x1b[?1049l".as_slice(),
        false => b"\x1b[?25h".as_slice(),
    });
    // SAFETY: the shell's own controlling terminal, open for as long as the process is.
    let handle = unsafe { std::os::fd::BorrowedFd::borrow_raw(saved.fd) };
    write_all(handle, &sequence);

    // Last, and the one that matters most: without it the user has no echo and no Ctrl-C.
    let _ = tcsetattr(handle, SetArg::TCSANOW, &saved.original);
}

/// `write` straight at the descriptor, retrying a short write and a signal.
///
/// No flush, because there is nothing buffered to flush — that is the point of writing here rather
/// than through `stderr()`. A failure is dropped: the terminal flags still get put back below, and
/// they are what the user actually needs.
fn write_all(fd: std::os::fd::BorrowedFd<'_>, mut bytes: &[u8]) {
    while !bytes.is_empty() {
        match nix::unistd::write(fd, bytes) {
            Ok(0) => break,
            Ok(n) => bytes = &bytes[n..],
            Err(nix::errno::Errno::EINTR) => continue,
            Err(_) => break,
        }
    }
}

/// Install the hook once, chaining to whatever was there.
///
/// Chained rather than replaced so the panic message still reaches the user — restoring the
/// terminal and then swallowing the reason would trade one silent death for another.
fn install_hook() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            give_it_back();
            previous(info);
        }));
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use nix::sys::termios::{LocalFlags, tcgetattr};

    /// **One slot for the process, so these take turns.** What was stashed is a single global —
    /// `forget` empties it and `give_it_back` drains it — so a test that remembers a terminal and
    /// then panics could have another test wipe the slot in between, and the hook would find
    /// nothing to put back. About one run in twenty.
    fn serialised() -> std::sync::MutexGuard<'static, ()> {
        static SERIAL: Mutex<()> = Mutex::new(());
        SERIAL
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// **The editor's flags are put back even when nothing unwinds.**
    ///
    /// `Drop for Restore` is the ordinary way home and `panic = "abort"` skips it, so a panic used
    /// to leave the user's terminal with `ICANON`, `ECHO` and `ISIG` cleared: nothing echoed, no
    /// line editing, Ctrl-C dead, and `reset` the only way out. This is the work the hook does.
    #[test]
    fn the_terminal_flags_come_back() {
        let _order = serialised();
        let pty = match nix::pty::openpty(None, None) {
            Ok(pty) => pty,
            // A machine with no pty to spare is not a failing shell.
            Err(_) => return,
        };
        use std::os::fd::AsRawFd;
        let fd = pty.slave.as_raw_fd();
        let handle = unsafe { std::os::fd::BorrowedFd::borrow_raw(fd) };

        let original = tcgetattr(handle).expect("a pty answers tcgetattr");
        assert!(original.local_flags.contains(LocalFlags::ECHO));

        // Take the terminal the way the editor does, and stash what it takes to undo that.
        let raw = super::super::editor_termios(&original);
        tcsetattr(handle, SetArg::TCSANOW, &raw).expect("set raw");
        remember(fd, &original, false, true, false, false);
        assert!(
            !tcgetattr(handle)
                .unwrap()
                .local_flags
                .contains(LocalFlags::ECHO),
            "the editor's terminal has no echo"
        );

        // What the panic hook does, without arranging a panic.
        give_it_back();
        let after = tcgetattr(handle).expect("tcgetattr");
        assert!(
            after.local_flags.contains(LocalFlags::ECHO)
                && after.local_flags.contains(LocalFlags::ICANON)
                && after.local_flags.contains(LocalFlags::ISIG),
            "echo, line editing and Ctrl-C are all back"
        );
    }

    /// Calling it twice is harmless, and calling it when the editor never took the terminal does
    /// nothing at all — a hook fires on every panic, including ones from a non-interactive shell.
    #[test]
    fn rescuing_nothing_is_not_an_error() {
        let _order = serialised();
        forget();
        give_it_back();
        give_it_back();
    }

    /// **The hook itself fires, not merely the function it calls.**
    ///
    /// The test above proves the restoration works; this proves it is wired to a panic. A test
    /// build unwinds rather than aborting, but a panic *hook* runs before either — which is the
    /// whole reason this mechanism can work under `panic = "abort"` at all.
    #[test]
    fn a_panic_puts_the_terminal_back() {
        let _order = serialised();
        let Ok(pty) = nix::pty::openpty(None, None) else {
            return;
        };
        use std::os::fd::AsRawFd;
        let fd = pty.slave.as_raw_fd();
        let handle = unsafe { std::os::fd::BorrowedFd::borrow_raw(fd) };

        let original = tcgetattr(handle).expect("tcgetattr");
        let raw = super::super::editor_termios(&original);
        tcsetattr(handle, SetArg::TCSANOW, &raw).expect("set raw");
        remember(fd, &original, false, false, false, false);

        // The hook runs on the way out of this, before the unwind reaches the caller.
        let panicked = std::panic::catch_unwind(|| panic!("the editor fell over"));
        assert!(panicked.is_err(), "the panic happened");

        let after = tcgetattr(handle).expect("tcgetattr");
        assert!(
            after.local_flags.contains(LocalFlags::ECHO)
                && after.local_flags.contains(LocalFlags::ISIG),
            "the panic hook gave the terminal back"
        );
    }
}

/// **A panic behind a redirect must not be a silent death.**
///
/// `.env.lua` evaluation points stdout and stderr at an unlinked scratch file so its output can be
/// shown tidily, and restores them *after* a `catch_unwind` that `panic = "abort"` never returns
/// from. So a panic anywhere in that evaluation wrote its message into a file nobody will ever read
/// and killed an interactive shell with a blank terminal.
#[cfg(test)]
mod redirect_tests {
    use super::*;
    use std::io::{Read, Seek, Write};
    use std::os::fd::AsRawFd;

    /// The hook puts a redirected descriptor back before the message is printed.
    #[test]
    fn a_held_descriptor_comes_back_on_a_panic() {
        let mut scratch = tempfile::tempfile().expect("a scratch file");
        let mut real = tempfile::tempfile().expect("somewhere to be restored to");

        // Stand in for stderr: a descriptor of our own, pointed at `real`, then moved to `scratch`
        // the way `capturing` moves the shell's.
        let target = nix::unistd::dup(real.as_raw_fd()).expect("a target to move");
        let saved = nix::unistd::dup(target).expect("the copy that puts it back");
        let _ = nix::unistd::dup2(scratch.as_raw_fd(), target);
        holding(target, saved);

        // What a panic would write goes to the target while it is redirected…
        let handle = unsafe { std::os::fd::BorrowedFd::borrow_raw(target) };
        write_all(handle, b"into the scratch file\n");

        let panicked = std::panic::catch_unwind(|| panic!("inside the redirect"));
        assert!(panicked.is_err(), "the panic happened");

        // …and after the hook, the same descriptor reaches the real file again.
        write_all(handle, b"onto the terminal\n");

        let mut said = String::new();
        real.seek(std::io::SeekFrom::Start(0)).expect("rewind");
        real.read_to_string(&mut said).expect("read");
        assert!(
            said.contains("onto the terminal"),
            "the descriptor was put back: {said:?}"
        );

        let mut hidden = String::new();
        scratch.seek(std::io::SeekFrom::Start(0)).expect("rewind");
        scratch.read_to_string(&mut hidden).expect("read");
        assert!(
            hidden.contains("into the scratch file"),
            "and the redirect was real while it lasted: {hidden:?}"
        );
        let _ = scratch.flush();
    }

    /// Releasing it the ordinary way leaves the hook nothing to do — and no stale descriptor number
    /// to dup2 over a later, unrelated redirect.
    #[test]
    fn a_released_descriptor_is_not_touched_again() {
        let real = tempfile::tempfile().expect("a file");
        let target = nix::unistd::dup(real.as_raw_fd()).expect("a target");
        let saved = nix::unistd::dup(target).expect("a copy");
        holding(target, saved);
        released(target);

        assert!(
            redirected()
                .lock()
                .is_ok_and(|held| { !held.iter().any(|(at, _)| *at == target) }),
            "it is no longer held"
        );
    }
}

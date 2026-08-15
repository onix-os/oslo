//! Waiting for the next thing, whichever direction it comes from.
//!
//! The editor's wait used to be one blocking `read` on the terminal, and everything the shell
//! learns while nobody is typing — a job that ended, a timer that came due, a universal variable
//! set in another window — arrives on a descriptor that is not the terminal. So the wait watches
//! them together and the caller still only hears about keys and markers.
//!
//! # Two waits, one set
//!
//! [`Keys::fill`] blocks until something happens; [`waiting`] does the same with a deadline, for a
//! prompt being rebuilt or an idle hook armed. **Both poll the same descriptors**, and the bug that
//! forced this file into existence was them disagreeing: whether a finished job was noticed
//! depended on which of the two the editor happened to be sitting in.

use super::Keys;
use crate::term::child::CHILD_MARK;
use crate::term::resize::{RESIZE_MARK, take_resize};
use oslo_base::background;
use std::io;

impl Keys {
    pub(super) fn fill(&mut self) -> bool {
        let mut byte = [0u8; 1];
        loop {
            // **Only when something is registered.** With no wakers this is the blocking read it
            // has always been, byte for byte — a script, a `sh -c` and a shell with nothing being
            // watched all take exactly the path they used to. The poll is the opt-in half.
            if !background::wakers().is_empty() {
                match self.wait_for_terminal_or_background() {
                    Waited::Marked => return true,
                    // Serviced and there was nothing to say. Wait again rather than fall through:
                    // the terminal has no data, and a read here would block with the wakers unwatched.
                    Waited::Nothing => continue,
                    Waited::Terminal => {}
                }
            }
            // SAFETY: the destination is one live byte and `fd` is owned by the caller.
            let n = unsafe { nix::libc::read(self.fd, byte.as_mut_ptr().cast(), 1) };
            if n > 0 {
                self.buf.push(byte[0]);
                return true;
            }
            if n == 0 {
                return false;
            }
            match io::Error::last_os_error().kind() {
                io::ErrorKind::Interrupted => {
                    if take_resize() {
                        self.buf.push(RESIZE_MARK);
                        return true;
                    }
                }
                io::ErrorKind::WouldBlock => {
                    let mut ready = nix::libc::pollfd {
                        fd: self.fd,
                        events: nix::libc::POLLIN,
                        revents: 0,
                    };
                    // SAFETY: one valid descriptor is polled without a timeout.
                    let polled = unsafe { nix::libc::poll(&mut ready, 1, -1) };
                    if polled < 0 {
                        if io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
                            return false;
                        }
                        if take_resize() {
                            self.buf.push(RESIZE_MARK);
                            return true;
                        }
                    }
                }
                _ => return false,
            }
        }
    }

    /// Wait until the terminal has something, or the background does.
    ///
    /// Answers `true` when it serviced the background and pushed a marker — the caller returns and
    /// the loop gets its turn to repaint. `false` means "the terminal is ready, go and read it",
    /// which is also what every error answers: a broken poll must fall through to the read rather
    /// than spin, and the read reports the real failure.
    ///
    /// **`poll` on a blocking descriptor is fine.** It reports when a read *would not* block; it
    /// does not require `O_NONBLOCK`, which is why the terminal does not have to change mode for
    /// any of this.
    fn wait_for_terminal_or_background(&mut self) -> Waited {
        let wakers = background::wakers();
        let mut fds = Vec::with_capacity(wakers.len() + 1);
        fds.push(nix::libc::pollfd {
            fd: self.fd,
            events: nix::libc::POLLIN,
            revents: 0,
        });
        for waker in &wakers {
            fds.push(nix::libc::pollfd {
                fd: *waker,
                events: nix::libc::POLLIN,
                revents: 0,
            });
        }
        // The nearest timer's deadline, or no timeout when nothing is waiting on the clock.
        let timeout = background::wait_ms();
        // SAFETY: every descriptor is borrowed and live for the duration of the call.
        let polled = unsafe { nix::libc::poll(fds.as_mut_ptr(), fds.len() as _, timeout) };
        if polled == 0 {
            // A deadline came due with nothing else to say. Servicing is what fires the timer.
            return match background::service() {
                true => {
                    self.buf.push(CHILD_MARK);
                    Waited::Marked
                }
                false => Waited::Nothing,
            };
        }
        if polled < 0 {
            // **A signal that arrives during the poll has already been and gone**, so it will not
            // interrupt the read below — the flags it set have to be read here or the shell goes
            // back to blocking with the news unread.
            if take_resize() {
                self.buf.push(RESIZE_MARK);
                return Waited::Marked;
            }
            // **`poll` is never restarted, `SA_RESTART` or not**, so a child ending fails the wait
            // it was meant to end. Answering `Terminal` here sent the shell into a blocking `read`
            // with its own wake byte still sitting unread in the pipe, and it stayed there until
            // somebody typed — the very thing this whole path exists to avoid. Waiting again is
            // free: whatever the handler wrote is still pending, so the next poll returns at once.
            return match io::Error::last_os_error().kind() {
                io::ErrorKind::Interrupted => Waited::Nothing,
                // Anything else is a broken descriptor set. Falling through to the read reports the
                // real failure rather than spinning on a poll that will keep failing.
                _ => Waited::Terminal,
            };
        }
        let mut woken = false;
        for (at, waker) in wakers.iter().enumerate() {
            if fds[at + 1].revents & (nix::libc::POLLIN | nix::libc::POLLHUP) != 0 {
                // Emptied whatever it held; `woken` only when what it held was interesting.
                woken |= background::read_waker(*waker);
            }
        }
        // **`Terminal` only when the terminal actually spoke.** Saying it otherwise sends `fill`
        // into a blocking `read` with the wakers no longer watched, and the next wake — a finished
        // job, a timer, a universal set elsewhere — lands in a pipe nobody is polling. One
        // uninteresting `inotify` event was enough to wedge the whole idle path for good, and it
        // took an incidental event to hit, which is why the tests around it disagreed with each
        // other.
        let spoke = fds[0].revents & (nix::libc::POLLIN | nix::libc::POLLHUP) != 0;
        if !woken {
            return match spoke {
                true => Waited::Terminal,
                false => Waited::Nothing,
            };
        }
        // **A repaint only when something actually changed.** A watch is on a *directory*, and the
        // shell writes its own history and state into the same one — so most wakes are the shell
        // hearing itself. Marking every one of them would repaint the prompt on every command.
        match background::service() {
            true => {
                self.buf.push(CHILD_MARK);
                Waited::Marked
            }
            false => Waited::Nothing,
        }
    }
}

/// What one wait produced.
enum Waited {
    /// Something was pushed into the buffer; the caller returns and the loop takes its turn.
    Marked,
    /// The terminal has data. Go and read it.
    Terminal,
    /// The background was serviced and had nothing to report. Wait again.
    Nothing,
}

/// Whether the terminal has something within `ms`.
///
/// **The background is watched here too, and answering `false` for it is deliberate.** This is the
/// *timed* wait — used while a prompt is being rebuilt or an idle hook is armed — and its callers
/// want to know about a keystroke. A wake that is not one is serviced and then reported as a
/// timeout, so the caller loops and waits again, exactly as it would have on a quiet terminal.
///
/// Without this the two waits disagreed: the blocking one watched the wake pipe and this one did
/// not, so whether a finished job was noticed depended on whether a prompt happened to be
/// refreshing — which is as good as random.
pub(super) fn waiting(fd: i32, ms: i32) -> bool {
    let wakers = background::wakers();
    let mut fds = Vec::with_capacity(wakers.len() + 1);
    fds.push(nix::libc::pollfd {
        fd,
        events: nix::libc::POLLIN,
        revents: 0,
    });
    for waker in &wakers {
        fds.push(nix::libc::pollfd {
            fd: *waker,
            events: nix::libc::POLLIN,
            revents: 0,
        });
    }
    // SAFETY: every descriptor is borrowed and live for the duration of the call.
    let polled = unsafe { nix::libc::poll(fds.as_mut_ptr(), fds.len() as _, ms) };
    if polled <= 0 {
        return false;
    }
    let mut woken = false;
    for (at, waker) in wakers.iter().enumerate() {
        if fds[at + 1].revents & (nix::libc::POLLIN | nix::libc::POLLHUP) != 0 {
            woken |= background::read_waker(*waker);
        }
    }
    if woken {
        background::service();
    }
    fds[0].revents & (nix::libc::POLLIN | nix::libc::POLLHUP) != 0
}

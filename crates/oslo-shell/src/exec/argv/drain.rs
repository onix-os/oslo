//! The pipes a captured command writes to, and reading them without deadlocking.
//!
//! Split from [`super`] so that file stays about *running* a command — fork, exec, wait, status —
//! and this one is about the bytes coming back. The two halves have different hazards: there it is
//! descriptor ownership and signal disposition, here it is that the obvious reader deadlocks.

use super::{Limit, pipe_failed};
use nix::poll::{PollFd, PollFlags, PollTimeout, poll};
use oslo_base::error::Result;

use std::os::fd::{AsFd, AsRawFd, OwnedFd};

/// **`O_CLOEXEC`**, because `nix`'s `pipe()` is a bare `libc::pipe` and sets nothing: every capture
/// pipe was inherited by whatever the stage went on to `exec`. The ends this shell means a child to
/// have are installed with `dup2`, which clears the flag on the copy, so nothing that should
/// survive the `exec` stops surviving it.
pub(super) fn pipe_pair() -> Result<(OwnedFd, OwnedFd)> {
    nix::unistd::pipe2(nix::fcntl::OFlag::O_CLOEXEC).map_err(pipe_failed)
}

/// Read both pipes until they close.
///
/// Polled rather than read one after the other. Reading stdout to the end first deadlocks the
/// moment a command writes more than a pipe buffer to stderr: the child blocks writing to a pipe
/// nobody is draining, and the parent blocks reading one the child will never finish.
pub(super) fn drain_until(
    out: Option<OwnedFd>,
    err: Option<OwnedFd>,
    child: nix::unistd::Pid,
    limit: Option<Limit>,
) -> (String, String, bool) {
    let Some(limit) = limit else {
        let (out, err) = drain(out, err);
        return (out, err, false);
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(limit.ms);
    let (out, err, hit) = drain_by(out, err, Some(deadline));
    if !hit {
        return (out, err, false);
    }
    // **The signal goes to the whole group**, which under a deadline is a group this function
    // created for exactly this purpose — see the `setpgid` in the child above.
    // The whole group: the child is oslo running a subshell, so the command is one level further
    // down. `setpgid` above is what makes this reach it.
    // The direct child first, then the group it leads. The child may have moved itself into
    // another group by now (job control does that on the exec path), so the group alone is not
    // enough — and the group alone was measured leaving `sleep 10` running to completion.
    let _ = nix::sys::signal::kill(child, limit.signal);
    let _ = nix::sys::signal::kill(nix::unistd::Pid::from_raw(-child.as_raw()), limit.signal);

    // A grace window to collect what it wrote before it died. Bounded, because a child that
    // *ignores* the signal would otherwise hang here — and a grandchild holding the write end
    // keeps the pipe open even after the direct child is gone, so waiting for EOF is not an option.
    // Whatever has not arrived by now is lost; `wait` below still reaps the child either way.
    let grace = std::time::Instant::now() + std::time::Duration::from_millis(250);
    let (more_out, more_err, _) = drain_by(None, None, Some(grace));
    let _ = (more_out, more_err);
    (out, err, true)
}

/// The drain loop, optionally bounded. `true` when the deadline passed with a stream still open.
pub(super) fn drain_by(
    out: Option<OwnedFd>,
    err: Option<OwnedFd>,
    deadline: Option<std::time::Instant>,
) -> (String, String, bool) {
    let mut slots = [Stream::new(out), Stream::new(err)];
    let mut expired = false;

    while slots.iter().any(Stream::open) {
        let wait_for = match deadline {
            None => PollTimeout::NONE,
            Some(at) => {
                let left = at.saturating_duration_since(std::time::Instant::now());
                if left.is_zero() {
                    expired = true;
                    break;
                }
                PollTimeout::try_from(left.as_millis().min(i32::MAX as u128) as i32)
                    .unwrap_or(PollTimeout::NONE)
            }
        };
        // **Which slot each descriptor belongs to, kept beside it.** The unbounded `drain` reads
        // *every* open slot once `poll` returns, which is safe only because it never has a deadline
        // to miss: with both streams captured and only one ready, the read on the other blocks
        // until the command exits. Under a deadline that is fatal — measured, a 300 ms limit on
        // `sh -c 'echo before; sleep 10'` returned after the full ten seconds — so this reads only
        // what `poll` reported.
        let watching: Vec<usize> = (0..slots.len()).filter(|i| slots[*i].open()).collect();
        let fds: Vec<PollFd> = watching
            .iter()
            .filter_map(|i| slots[*i].fd.as_ref())
            .map(|fd| PollFd::new(fd.as_fd(), PollFlags::POLLIN))
            .collect();
        let mut fds = fds;
        match poll(&mut fds, wait_for) {
            Err(_) => break,
            // Nothing was ready before the timeout, which is the deadline passing.
            Ok(0) if deadline.is_some() => {
                expired = true;
                break;
            }
            Ok(_) => {}
        }
        let ready: Vec<usize> = watching
            .iter()
            .zip(fds.iter())
            .filter(|(_, fd)| {
                fd.revents()
                    .is_some_and(|r| r.intersects(PollFlags::POLLIN | PollFlags::POLLHUP))
            })
            .map(|(slot, _)| *slot)
            .collect();
        for slot in ready {
            slots[slot].read_once();
        }
    }

    let [out, err] = slots;
    (text(&out.buffer), text(&err.buffer), expired)
}

pub(super) fn drain(out: Option<OwnedFd>, err: Option<OwnedFd>) -> (String, String) {
    // A stream that was never captured starts already finished, which keeps the loop below from
    // having to care how many there are.
    let mut slots = [Stream::new(out), Stream::new(err)];

    while slots.iter().any(Stream::open) {
        let fds: Vec<PollFd> = slots
            .iter()
            .filter(|s| s.open())
            .filter_map(|s| s.fd.as_ref())
            .map(|fd| PollFd::new(fd.as_fd(), PollFlags::POLLIN))
            .collect();
        if poll(&mut { fds }, PollTimeout::NONE).is_err() {
            break;
        }
        for slot in slots.iter_mut().filter(|s| s.open()) {
            slot.read_once();
        }
    }

    let [out, err] = slots;
    (text(&out.buffer), text(&err.buffer))
}

/// One captured stream, and what has been read from it.
struct Stream {
    fd: Option<OwnedFd>,
    buffer: Vec<u8>,
    finished: bool,
}

impl Stream {
    fn new(fd: Option<OwnedFd>) -> Self {
        Stream {
            finished: fd.is_none(),
            fd,
            buffer: Vec::new(),
        }
    }

    fn open(&self) -> bool {
        !self.finished
    }

    fn read_once(&mut self) {
        let Some(fd) = &self.fd else {
            self.finished = true;
            return;
        };
        let mut chunk = [0u8; 8192];
        match nix::unistd::read(fd.as_raw_fd(), &mut chunk) {
            Ok(0) => self.finished = true,
            Ok(n) => self.buffer.extend_from_slice(&chunk[..n]),
            // Interrupted before any bytes moved; ask again.
            Err(nix::errno::Errno::EINTR) => {}
            Err(_) => self.finished = true,
        }
    }
}

/// Captured bytes as a string, with the trailing newline removed.
///
/// Matching `$(cmd)`: the shell's own capture strips it, and a script comparing against `"x"`
/// should not have to remember the command printed `"x\n"`.
pub(super) fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .trim_end_matches('\n')
        .to_string()
}

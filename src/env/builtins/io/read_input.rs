//! The byte-level half of `read`: which descriptor, which delimiter, how many characters, and
//! how long to wait for them.
//!
//! Every read here is one byte long, deliberately. A buffered reader would consume input past
//! its own delimiter, and the next command sharing the descriptor (`{ read x; cat; } < f`) would
//! find the file already drained. That is why bash reads a byte at a time on anything it cannot
//! seek, and why `io::stdin()` — a process-global 8 KiB `BufReader` — must never be touched
//! here.

use nix::errno::Errno;
use nix::libc;
use std::os::fd::{BorrowedFd, RawFd};
use std::time::{Duration, Instant};

/// Why the read stopped. Only this distinguishes a complete line from a truncated one, which is
/// the entire basis of `read`'s exit status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stop {
    /// The delimiter arrived and was consumed.
    Delimiter,
    /// `-n`/`-N` were satisfied without one.
    Budget,
    /// Input ran out first. bash reports failure here *even when data was read* — that is what
    /// makes `while read l; do …; done < file` terminate.
    Eof,
    /// `-t` expired. Whatever arrived first is still assigned.
    TimedOut,
}

/// One logical line as `read` sees it: the delimiter is gone and backslash escapes have been
/// resolved, unless `-r` was given.
pub struct InputLine {
    /// Line content with the escaping backslashes themselves removed.
    pub bytes: Vec<u8>,
    /// Parallel to `bytes`: a byte that arrived escaped can never act as a field delimiter.
    pub escaped: Vec<bool>,
    pub stop: Stop,
    /// Characters, not bytes: `-n`/`-N` count what the user typed, and a UTF-8 continuation
    /// byte is not a character. Counting bytes would cut a multi-byte character in half.
    chars: usize,
    /// Continuation bytes still owed by the character being assembled. The `-n` budget is only
    /// consulted at zero, so a limit can never land mid-character.
    pending_bytes: usize,
}

/// How many continuation bytes a UTF-8 leading byte promises. Zero for ASCII, and zero for a
/// stray continuation byte — malformed input must not stall the counter.
fn continuation_len(byte: u8) -> usize {
    match byte {
        0xc0..=0xdf => 1,
        0xe0..=0xef => 2,
        0xf0..=0xf7 => 3,
        _ => 0,
    }
}

impl InputLine {
    fn push(&mut self, byte: u8, escaped: bool) {
        // A continuation byte belongs to the character its leading byte started, so it inherits
        // that character's escaped-ness: `\é` must be one escaped character, not a quoted byte
        // followed by two unquoted ones.
        let escaped = if self.pending_bytes > 0 {
            self.escaped.last().copied().unwrap_or(escaped)
        } else {
            escaped
        };
        self.bytes.push(byte);
        self.escaped.push(escaped);
        if self.pending_bytes > 0 {
            self.pending_bytes -= 1;
        } else {
            self.chars += 1;
            self.pending_bytes = continuation_len(byte);
        }
    }

    /// Whether a `-n`/`-N` budget has been filled. Never true part-way through a character.
    fn budget_filled(&self, limit: Option<usize>) -> bool {
        self.pending_bytes == 0 && limit.is_some_and(|limit| self.chars >= limit)
    }

    /// A line as if `text` had been read with nothing escaped — for tests of the split half,
    /// which needs a line without a descriptor to read it from.
    #[cfg(test)]
    pub fn from_text(text: &str) -> Self {
        let mut line = InputLine {
            bytes: Vec::new(),
            escaped: Vec::new(),
            stop: Stop::Delimiter,
            chars: 0,
            pending_bytes: 0,
        };
        for byte in text.as_bytes() {
            line.push(*byte, false);
        }
        line
    }

    /// Mark the byte at `index` as having arrived escaped.
    #[cfg(test)]
    pub fn mark_escaped(&mut self, index: usize) {
        self.escaped[index] = true;
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.bytes).into_owned()
    }
}

/// Everything the option grammar decided before a single byte was read.
pub struct InputSpec {
    pub fd: RawFd,
    /// `-r`: backslashes are data, not escapes.
    pub raw: bool,
    /// The byte that ends the line, or `None` under `-N`, which reads through delimiters.
    pub delim: Option<u8>,
    /// `-n`/`-N`: stop after this many resulting characters.
    pub limit: Option<usize>,
    /// `-t`, in seconds.
    pub timeout: Option<f64>,
    /// `-s`: do not echo what a terminal sends back.
    pub silent: bool,
}

/// Wait until `fd` has input or the deadline passes. `false` means the deadline won.
///
/// `poll` rather than a blocking read: `read -t` has to give up on a descriptor nobody is
/// writing to, and there is no way to un-block a read once it has started.
fn wait_readable(fd: RawFd, deadline: Instant) -> Result<bool, Errno> {
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let millis = remaining.as_millis().min(i32::MAX as u128) as i32;
        let mut pfd = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: one initialised `pollfd`, and the length matches the slice we pass.
        let n = unsafe { libc::poll(&mut pfd, 1, millis) };
        if n < 0 {
            let err = Errno::last();
            if err == Errno::EINTR {
                // A caught signal is not a timeout; recompute the budget and keep waiting.
                continue;
            }
            return Err(err);
        }
        return Ok(n > 0);
    }
}

/// `-t 0`: whether input is available *without consuming any*.
///
/// A zero timeout is a probe, not a very short read: bash documents it as returning success when
/// the descriptor has input, and a `read` that swallowed a byte to find that out would be
/// useless for the polling loops the option exists to serve.
pub fn probe_readable(fd: RawFd) -> Result<bool, Errno> {
    wait_readable(fd, Instant::now())
}

/// Terminal echo suppressed for the lifetime of the guard (`-s`).
///
/// Restoring on drop matters more than the feature does: a `read -s` that returns early — EOF,
/// timeout, a signal — must not leave the user's terminal unable to show what they type.
struct EchoOff {
    fd: RawFd,
    saved: Option<nix::sys::termios::Termios>,
}

impl EchoOff {
    fn engage(fd: RawFd, silent: bool) -> Self {
        let mut guard = EchoOff { fd, saved: None };
        if !silent {
            return guard;
        }
        // SAFETY: borrowed for this call only; the descriptor outlives it.
        let borrowed = unsafe { BorrowedFd::borrow_raw(fd) };
        let Ok(saved) = nix::sys::termios::tcgetattr(borrowed) else {
            // Not a terminal: there is no echo to suppress and nothing to restore.
            return guard;
        };
        let mut quiet = saved.clone();
        quiet
            .local_flags
            .remove(nix::sys::termios::LocalFlags::ECHO);
        if nix::sys::termios::tcsetattr(borrowed, nix::sys::termios::SetArg::TCSANOW, &quiet)
            .is_ok()
        {
            guard.saved = Some(saved);
        }
        guard
    }
}

impl Drop for EchoOff {
    fn drop(&mut self) {
        if let Some(saved) = self.saved.take() {
            let borrowed = unsafe { BorrowedFd::borrow_raw(self.fd) };
            let _ =
                nix::sys::termios::tcsetattr(borrowed, nix::sys::termios::SetArg::TCSANOW, &saved);
        }
    }
}

/// Read one logical line from `spec.fd`.
pub fn read_logical_line(spec: &InputSpec) -> Result<InputLine, Errno> {
    let mut line = InputLine {
        bytes: Vec::new(),
        escaped: Vec::new(),
        stop: Stop::Eof,
        chars: 0,
        pending_bytes: 0,
    };
    let _echo = EchoOff::engage(spec.fd, spec.silent);
    let deadline = spec
        .timeout
        .map(|secs| Instant::now() + Duration::from_secs_f64(secs.max(0.0)));

    // `-n 0` is satisfied before any input is needed, which is how `read -t 0` and `read -n 0`
    // probe a descriptor without disturbing it.
    if spec.limit == Some(0) {
        line.stop = Stop::Budget;
        return Ok(line);
    }

    let mut pending_escape = false;
    let mut buf = [0u8; 1];
    loop {
        if let Some(deadline) = deadline
            && !wait_readable(spec.fd, deadline)?
        {
            line.stop = Stop::TimedOut;
            return Ok(line);
        }

        let n = match nix::unistd::read(spec.fd, &mut buf) {
            Ok(n) => n,
            Err(Errno::EINTR) => continue,
            Err(e) => return Err(e),
        };
        if n == 0 {
            // A backslash with nothing behind it escapes nothing and is simply dropped, as in
            // bash: `printf 'a\' | read x` leaves `x=a`.
            line.stop = Stop::Eof;
            return Ok(line);
        }

        let byte = buf[0];
        if pending_escape {
            pending_escape = false;
            // Backslash-delimiter is a line continuation: both characters vanish and the line
            // keeps growing. Under `-N` there is no delimiter, so newline continues by default.
            if byte != spec.delim.unwrap_or(b'\n') {
                line.push(byte, true);
            }
        } else if spec.delim == Some(byte) {
            line.stop = Stop::Delimiter;
            return Ok(line);
        } else if byte == b'\\' && !spec.raw {
            pending_escape = true;
        } else {
            line.push(byte, false);
        }

        if line.budget_filled(spec.limit) {
            line.stop = Stop::Budget;
            return Ok(line);
        }
    }
}

/// The exit status a stop reason produces.
///
/// A timeout reports 128 + `SIGALRM`, which is what bash's alarm-driven implementation leaves
/// behind and what scripts test for.
pub fn status_of(stop: Stop) -> i32 {
    match stop {
        Stop::Delimiter | Stop::Budget => 0,
        Stop::Eof => 1,
        Stop::TimedOut => 128 + libc::SIGALRM,
    }
}

/// Whether `fd` is a terminal, which is the only case in which `-p` prints its prompt.
pub fn is_terminal(fd: RawFd) -> bool {
    nix::unistd::isatty(fd).unwrap_or(false)
}

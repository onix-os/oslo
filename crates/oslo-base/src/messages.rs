//! What this session said, kept so it can be read after it has scrolled.
//!
//! ```text
//! messages              everything, oldest first
//! messages -n 5         the last five
//! messages plugin       only what the plugin loader said
//! ```
//!
//! # Why a shell needs one
//!
//! A diagnostic is printed once, into a terminal that is a scrollback and a `clear` away from
//! losing it. That was survivable when the only things that spoke were the shell itself; a session
//! now loads `conf.d/*.lua`, `init.lua`, plugins, prompt segments and timers, and every one of
//! them can fail in a single line twenty commands ago. Neovim keeps `:messages` for exactly this.
//!
//! # In memory, and only this process
//!
//! What a session said is a session-lived fact. A file would need rotating, permissioning, and
//! would eventually hold a line somebody did not mean to keep — a hook that echoed a token into a
//! warning writes it to disk forever. The buffer is a ring: the oldest goes when it is full, so a
//! session that runs for a week cannot grow without bound.
//!
//! **This is also why the reader is a builtin and not `oslo messages`.** A tool runs in a new
//! process, which has said nothing; only the session that produced them can be asked.

use std::collections::VecDeque;
use std::sync::Mutex;

/// How many are kept. Enough that a failure at startup is still there after a long session, small
/// enough that the whole thing is one screen's worth of scrolling to read.
const KEEP: usize = 500;

/// How much of a session's own noise a message is.
///
/// Not a syslog severity: a shell has no `emerg`. Three levels are what a reader actually sorts by
/// — *this went wrong*, *this is worth knowing*, *this is the shell narrating itself*.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Level {
    Error,
    Warn,
    Note,
}

impl Level {
    pub fn word(self) -> &'static str {
        match self {
            Level::Error => "error",
            Level::Warn => "warn",
            Level::Note => "note",
        }
    }
}

/// One line, and what produced it.
///
/// **The source is the point.** "could not open the database" is a mystery; "plugin/notes: could
/// not open the database" is a bug report. Callers pass a name they already have — the plugin, the
/// config file, the hook — so no caller has to invent one.
#[derive(Clone, Debug)]
pub struct Message {
    pub level: Level,
    pub source: String,
    pub text: String,
    /// Seconds since the session started, not a wall clock. What a reader wants is the *order* and
    /// roughly how far in, and an elapsed number needs no timezone and no formatting.
    pub at: f64,
    /// How many times in a row this exact line was said.
    ///
    /// **Without this the buffer is a liability.** A prompt segment that raises fails on every
    /// draw, so pressing Return five hundred times would evict every other message with five
    /// hundred copies of one — the ring would reliably lose exactly the startup failure it exists
    /// to keep.
    pub times: u32,
}

static SAID: Mutex<VecDeque<Message>> = Mutex::new(VecDeque::new());
static STARTED: Mutex<Option<std::time::Instant>> = Mutex::new(None);

/// Seconds since the first message, treating the first as zero.
///
/// The session's real start is in `main`, which is below nothing and would have to reach up here to
/// set it. The first thing said is close enough for the only question this answers: was that at
/// startup, or just now?
fn elapsed() -> f64 {
    let mut started = match STARTED.lock() {
        Ok(started) => started,
        Err(poisoned) => poisoned.into_inner(),
    };
    let at = started.get_or_insert_with(std::time::Instant::now);
    at.elapsed().as_secs_f64()
}

/// Remember one. Does not print it — the caller decides whether it is also worth interrupting for.
pub fn say(level: Level, source: impl Into<String>, text: impl Into<String>) {
    let message = Message {
        level,
        source: source.into(),
        text: text.into(),
        at: elapsed(),
        times: 1,
    };
    let Ok(mut said) = SAID.lock() else {
        // A poisoned buffer means a panic while somebody held it. Losing a diagnostic is not worth
        // a second panic on top of the first.
        return;
    };
    // Said again with nothing in between: count it rather than keep it twice, and let `at` move to
    // the latest so the line reads as *still* happening rather than as history.
    if let Some(last) = said.back_mut()
        && last.level == message.level
        && last.source == message.source
        && last.text == message.text
    {
        last.times = last.times.saturating_add(1);
        last.at = message.at;
        return;
    }
    if said.len() >= KEEP {
        said.pop_front();
    }
    said.push_back(message);
}

/// Remember it *and* print it to stderr, which is what most callers were already doing.
///
/// The two are one call because the failure mode is a caller that keeps only one of them: a warning
/// that prints and is not kept cannot be read back, and one that is kept and does not print is
/// invisible at the moment it matters.
pub fn warn(source: impl Into<String>, text: impl Into<String>) {
    let text = text.into();
    let source = source.into();
    eprintln!("oslo: {source}: {text}");
    say(Level::Warn, source, text);
}

/// The same, for something that failed rather than something odd.
pub fn error(source: impl Into<String>, text: impl Into<String>) {
    let text = text.into();
    let source = source.into();
    eprintln!("oslo: {source}: {text}");
    say(Level::Error, source, text);
}

/// Everything said so far, oldest first.
pub fn all() -> Vec<Message> {
    match SAID.lock() {
        Ok(said) => said.iter().cloned().collect(),
        Err(poisoned) => poisoned.into_inner().iter().cloned().collect(),
    }
}

/// Forget everything. For a session that has read them and wants the next failure to stand alone.
pub fn clear() {
    if let Ok(mut said) = SAID.lock() {
        said.clear();
    }
}

#[cfg(test)]
#[path = "messages/tests.rs"]
mod tests;

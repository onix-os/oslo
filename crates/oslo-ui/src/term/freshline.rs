//! Making sure a prompt starts on a row of its own.
//!
//! # The output a prompt used to eat
//!
//! A command's output goes from the command straight to the terminal; the shell never sees it. So
//! when a command ends **without a trailing newline** — `printf x`, a `curl` whose body has none,
//! anything ending mid-row — the cursor is left partway along a row that has text on it, and the
//! next prompt is drawn from exactly there. The prompt block is written over that row, and the
//! output is gone. Not scrolled, not hidden: overwritten, with nothing to say it ever existed.
//!
//! ```text
//!   $ printf ONELINE          the command writes 7 characters and no newline
//!   ONELINE▊                  cursor sits at column 8 of that row
//!   $ ▊                       the next prompt is drawn from column 0 of the same row
//! ```
//!
//! # Why the shell has to ask
//!
//! There is no way to work it out. The bytes did not pass through oslo, the width is the
//! terminal's, and a program may have moved the cursor itself. Every shell answers this the same
//! way — by finding out where the cursor is — and the only real choice is *how*.
//!
//! zsh and fish print a marker padded to the width of the row so the terminal wraps, which needs no
//! reply but spends a blank row every time the cursor was already at column 0 — which is the common
//! case. oslo asks instead: one `DSR` per prompt, and only where a terminal answers it at all.
//!
//! # Asked once about the terminal, not once about the question
//!
//! A terminal that does not answer the query never will, so the first silence is remembered and
//! nothing is asked again. That turns the cost of an unusual terminal from 60 ms per prompt into
//! 60 ms per session, and leaves it behaving exactly as it did before this existed.

use std::sync::atomic::{AtomicU8, Ordering};

/// **The standard cursor report, not the private one.** `ESC[6n` is answered by everything back to
/// a VT100; `ESC[?6n` is DECXCPR, which adds a page number nobody needs and which tmux — among
/// others — does not answer at all. `term::mouse` asks the private one, which is why this does not
/// borrow it: the query that came back empty in a multiplexer is exactly the query that would have
/// left this doing nothing.
const QUERY: &[u8] = b"\x1b[6n";

/// The reply comes from the terminal itself, so a terminal that answers answers immediately. This
/// is the budget for finding out that one does not.
const TIMEOUT_MS: u64 = 60;

/// What is known about this terminal: `0` not asked, `1` does not answer, `2` answers.
static ANSWERS: AtomicU8 = AtomicU8::new(0);

const UNKNOWN: u8 = 0;
const SILENT: u8 = 1;
const REPLIES: u8 = 2;

/// Put the cursor on a fresh row if something left it partway along one.
///
/// Answers the input bytes that arrived while waiting, which belong to the caller's key reader —
/// dropping them would swallow whatever was typed during the query.
pub fn ensure(fd: i32) -> Vec<u8> {
    if ANSWERS.load(Ordering::Relaxed) == SILENT {
        return Vec::new();
    }
    let (reply, pending) = super::query::query_sequence(fd, QUERY, TIMEOUT_MS, classify);
    let column = reply.as_deref().and_then(parse_column);
    let Some(column) = column else {
        // Silence is an answer about the terminal, and it is the same answer every time.
        ANSWERS.store(SILENT, Ordering::Relaxed);
        return pending;
    };
    ANSWERS.store(REPLIES, Ordering::Relaxed);
    // The reply is 1-based: column 1 is the left margin, and a prompt drawn there covers nothing.
    if column > 1 {
        use std::io::Write;
        let mut out = std::io::stderr();
        let _ = out.write_all(b"\r\n");
        let _ = out.flush();
    }
    pending
}

/// Whether the bytes so far are, or could still become, a cursor report.
fn classify(bytes: &[u8]) -> super::query::ReplyMatch {
    use super::query::ReplyMatch;
    if b"\x1b[".starts_with(bytes) {
        return ReplyMatch::Prefix;
    }
    if !bytes.starts_with(b"\x1b[") {
        return ReplyMatch::Reject;
    }
    match bytes.last() {
        Some(b'R') => match parse_column(bytes) {
            Some(_) => ReplyMatch::Complete,
            None => ReplyMatch::Reject,
        },
        Some(byte) if byte.is_ascii_digit() || *byte == b';' => ReplyMatch::Prefix,
        _ => ReplyMatch::Reject,
    }
}

/// The column out of `ESC [ row ; col R`, as the terminal counts it — from 1.
fn parse_column(bytes: &[u8]) -> Option<usize> {
    let body = std::str::from_utf8(bytes)
        .ok()?
        .strip_prefix("\x1b[")?
        .strip_suffix('R')?;
    let (row, column) = body.split_once(';')?;
    // **The row is checked even though it is thrown away.** Without it `ESC[?1;1R` parses — the
    // `?` rides along in the row field and the column still reads as 1 — so a stray reply to the
    // *private* query, which is a question this never asked, would be believed as an answer.
    if row.is_empty() || !row.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    column.parse::<usize>().ok()
}

/// Forget what was learned about this terminal. For tests, and for a session that has been handed
/// a different one.
pub fn forget() {
    ANSWERS.store(UNKNOWN, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **A terminal is asked once.** The whole cost argument rests on this: one silent query per
    /// session rather than one per prompt.
    #[test]
    fn silence_is_remembered() {
        forget();
        assert_eq!(ANSWERS.load(Ordering::Relaxed), UNKNOWN);
        ANSWERS.store(SILENT, Ordering::Relaxed);
        // No descriptor is touched once the answer is known, so this cannot block on a bad fd.
        assert!(ensure(-1).is_empty());
        forget();
    }

    /// The column is what the whole thing turns on, and `1` is the one value that means "do
    /// nothing" — an off-by-one here is a blank row before every prompt.
    #[test]
    fn a_report_gives_up_its_column() {
        assert_eq!(parse_column(b"\x1b[1;1R"), Some(1));
        assert_eq!(parse_column(b"\x1b[12;34R"), Some(34));
        assert_eq!(parse_column(b"\x1b[5;120R"), Some(120));
    }

    /// **Not the private form.** `ESC[?…R` is DECXCPR's answer, and reading it here would mean
    /// believing a reply to a question this never asked.
    #[test]
    fn anything_that_is_not_a_cursor_report_is_refused() {
        assert_eq!(parse_column(b"\x1b[?1;1R"), None);
        assert_eq!(parse_column(b"\x1b[1R"), None);
        assert_eq!(parse_column(b"\x1b[1;R"), None);
        assert_eq!(parse_column(b"\x1b[1;1"), None);
        assert_eq!(parse_column(b"nonsense"), None);
    }

    /// A reply arrives a byte at a time, so half of one must read as "keep waiting" rather than as
    /// a refusal — a classifier that rejected a prefix would time out on every terminal.
    #[test]
    fn a_half_read_report_is_still_pending() {
        use super::super::query::ReplyMatch;
        for partial in [
            b"\x1b".as_slice(),
            b"\x1b[".as_slice(),
            b"\x1b[1".as_slice(),
            b"\x1b[1;".as_slice(),
            b"\x1b[1;1".as_slice(),
        ] {
            assert_eq!(classify(partial), ReplyMatch::Prefix, "{partial:?}");
        }
        assert_eq!(classify(b"\x1b[1;1R"), ReplyMatch::Complete);
        assert_eq!(classify(b"\x1b[1;1X"), ReplyMatch::Reject);
        assert_eq!(classify(b"hello"), ReplyMatch::Reject);
    }
}

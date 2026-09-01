//! Saying a diagnostic once, in whichever of its two faces the reader can use.
//!
//! [`origin_now`](super::origin_now) answers *where* a diagnostic is speaking from. This answers
//! *how* it is drawn: a one-line message to a pipe, and on a terminal the same message with a caret
//! under the word at fault.
//!
//! ```text
//! oslo: kill: NOPE: invalid signal specification      ← a pipe, a script, a test
//!
//! oslo: kill: NOPE: invalid signal specification      ← a terminal
//!    ╭─[ kill:1:9 ]
//!  1 │ kill -s NOPE 1
//!    │         ──┬─
//!    │           ╰─── not a signal
//!    │ Help: a signal is a name (TERM), a number (15), or SIG-prefixed
//! ───╯
//! ```
//!
//! # One call, not five
//!
//! There are two hundred and fifty diagnostics in the builtins alone, and every one of them is
//! today a single `eprintln!`. If converting one meant five lines — ask whether to draw, build a
//! snapshot, find the word, build a report, fall back — then converting them all would be a
//! thousand lines of the same five, and the two hundred and fifty-first would be written the old
//! way because the new way is a chore.
//!
//! So it is one call with the same shape as the `eprintln!` it replaces, and the fallback is inside
//! it. A caller that has nothing to point at keeps its `eprintln!`; that is a decision about the
//! error, not an omission.
//!
//! # What `body` is
//!
//! Everything after the origin — `kill: NOPE: invalid signal specification` — exactly as the
//! `eprintln!` wrote it. **The message a pipe sees is byte-for-byte what it saw before**, which is
//! what `tests/diagnostics_stay_plain.rs` exists to hold true, and it is also the report's own
//! first line, so the two faces cannot drift into saying different things.

use super::origin_now;
use oslo_base::diag;

/// The one-liner, with a caret under `word` when there is a terminal to draw one on.
///
/// `words` is the command as the shell has it — the name and its operands. `word` is the one at
/// fault; when it is not among them the report is skipped and the one-liner printed, which is the
/// right answer for a word the message rewrote on its way there.
pub fn complain(words: &[String], word: &str, body: &str, label: &str, help: Option<&str>) {
    let message = format!("{}{body}", origin_now());
    if drawn(words, word, &message, label, help) {
        return;
    }
    eprintln!("{message}");
}

/// The same, for a caret under part of a word: `cols a,b,nmae` under `nmae` alone.
///
/// `inside` is a byte range within `word`.
pub fn complain_within(
    words: &[String],
    word: &str,
    inside: std::ops::Range<usize>,
    body: &str,
    label: &str,
    help: Option<&str>,
) {
    let message = format!("{}{body}", origin_now());
    let report = diag::Report {
        message: &message,
        source: source_of(words),
        label,
        help,
    };
    if diag::enabled() {
        let snapshot = diag::Snapshot::of(words);
        if let Some(at) = snapshot.index_of(word)
            && snapshot.draw_within(at, inside, &report)
        {
            return;
        }
    }
    eprintln!("{message}");
}

/// Whether a report was drawn. Split out so both entry points ask the question the same way.
fn drawn(words: &[String], word: &str, message: &str, label: &str, help: Option<&str>) -> bool {
    // **Asked before anything is built.** On a pipe this is the whole cost of the feature: one
    // cached bool, no snapshot, no format beyond the message that was going to be printed anyway.
    if !diag::enabled() {
        return false;
    }
    let snapshot = diag::Snapshot::of(words);
    let Some(at) = snapshot.index_of(word) else {
        return false;
    };
    snapshot.draw(
        at,
        &diag::Report {
            message,
            source: source_of(words),
            label,
            help,
        },
    )
}

/// What the report calls the line it is pointing into: the command's own name.
///
/// A builtin's argv is not a file, and pretending it has a path would be a lie in the one place a
/// person looks for one. `kill:1:9` reads as "the ninth column of what you typed", which is what it
/// is.
fn source_of(words: &[String]) -> &str {
    words.first().map(String::as_str).unwrap_or("oslo")
}

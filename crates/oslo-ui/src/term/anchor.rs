//! Asking the terminal where this prompt block begins.
//!
//! # The number a shell cannot know
//!
//! A prompt is redrawn by going back up to its first row and writing the whole block again, so
//! every redraw needs one number: how many rows above the cursor that first row is. The editor
//! keeps it by counting — it drew the block, so it knows how tall it was and where in it the
//! cursor ended up — and while the editor is the only thing writing to the screen, counting is
//! exact and costs nothing.
//!
//! It stops being exact the moment something else writes. A browser opened in a mux float, a
//! command that scrolled the screen, a prompt with rows added above it or below it: the count is
//! now a guess, and a guess one out either erases somebody else's line or leaves a stale copy of
//! the prompt behind. Patching the arithmetic case by case does not converge, because the shell
//! genuinely does not have the information.
//!
//! # So ask the one party that does
//!
//! `OSC 133;A` already marks the row the prompt starts on, and the terminal keeps that mark
//! however much has happened since — that is what it is for. It is not a mark a shell can read
//! back: `OSC 133` is write-only, and no standard report asks this question. So oslo asks with a
//! private DSR of its own, and a terminal that understands it answers from the mark.
//!
//! This is the same division of labour kitty settled on for redrawing prompts across a resize: the
//! terminal is the one that can see the screen, so the terminal owns the position and the shell
//! owns the drawing. The only difference is direction — kitty pushes (erases the block and signals
//! the shell), and this pulls, which needs no signal and cannot race the redraw it is for.
//!
//! # What happens where nothing answers
//!
//! Nothing, which is the point. An ordinary terminal does not recognise the request and stays
//! silent; the query times out — see `TIMEOUT_MS` — and the caller keeps the number it counted. So
//! this is an improvement where it is understood and costs a fifth of a frame where it is not, and
//! it is asked only at the boundaries where counting is known to be unreliable — never per
//! keystroke.

/// Oslo's number, the one the transcript marks already use. Written into a private DSR rather than
/// an OSC because this is a question with an answer, and `CSI ? n` is where a terminal's answers
/// about itself live.
const QUERY: &[u8] = b"\x1b[?1440n";

/// How long to wait for a terminal that has probably never heard of the question.
///
/// Short on purpose. The caller has a number already and is only hoping for a better one.
const TIMEOUT_MS: u64 = 20;

/// How many rows above the cursor the prompt block begins, and any input read while waiting.
///
/// `None` is both "nothing answered" and "answered, but no prompt is marked" — after a `clear`
/// there is no mark to measure from, and the terminal says so rather than leaving the caller to
/// time out. Either way the caller falls back to what it counted.
pub fn rows_above(fd: i32) -> (Option<usize>, Vec<u8>) {
    let (reply, pending) = super::query::query_sequence(fd, QUERY, TIMEOUT_MS, anchor_reply);
    (reply.as_deref().and_then(parse_anchor), pending)
}

fn anchor_reply(bytes: &[u8]) -> super::query::ReplyMatch {
    use super::query::ReplyMatch;
    let head = b"\x1b[?1440;";
    if head.starts_with(bytes) {
        return ReplyMatch::Prefix;
    }
    if !bytes.starts_with(head) {
        return ReplyMatch::Reject;
    }
    match bytes.last() {
        Some(b'n') => ReplyMatch::Complete,
        Some(byte) if byte.is_ascii_digit() => ReplyMatch::Prefix,
        _ => ReplyMatch::Reject,
    }
}

/// The reply is offset by one so that zero can mean "no prompt is marked".
fn parse_anchor(bytes: &[u8]) -> Option<usize> {
    std::str::from_utf8(bytes)
        .ok()?
        .strip_prefix("\x1b[?1440;")?
        .strip_suffix('n')?
        .parse::<usize>()
        .ok()?
        .checked_sub(1)
}

#[cfg(test)]
#[path = "anchor/tests.rs"]
mod tests;

//! The stack of streams a coordinate reads from, and substituting one into a command's words.
//!
//! A *stream* is text something produced: a pipeline stage that has finished, or a whole command at
//! the prompt. Both go on the same stack, so stepping back through a pipeline and stepping back
//! through the session are the same motion — see [`oslo_base::coords`] for the coordinate itself.
//!
//! # Which way the index goes
//!
//! ```text
//! cat hosts.txt | grep web | ssh {0:0}
//!                      │           └─ 0  this stage's input: what `grep web` printed
//!                      └───────────── 1  one stage further back: what `cat` printed
//!
//! ssh {-1:0:0}   ← -1  the previous prompt, whatever it was
//! ```
//!
//! **Zero and up walk back through this pipeline; below zero walks back through the session.** They
//! are different collections and giving them one axis would mean `{3:…}` silently crossing from one
//! into the other when a pipeline happened to be short. The sign says which you meant, and it reads
//! the way negative indices already read on a line: from the other end.
//!
//! # A value is one argument
//!
//! Every substituted value is `Origin::Quoted`, so a line holding a space, a `*` or a `$` arrives
//! at the command whole and is never re-split or re-globbed. This is the same guarantee
//! `expand::sugar` gives a marked directory's expansion, and for the same reason: a shell that
//! field-splits its own substitutions is a shell that executes filenames.

use crate::expand::word::{Origin, Run};
use oslo_base::coords::{self, Coord};

/// How many prompts back a coordinate may reach.
///
/// Ten is the number of things anybody keeps in their head; past that you look at the screen.
pub const PROMPTS_KEPT: usize = 10;

/// The most of one stream that is kept, in bytes.
///
/// Shared with `keep`/`copy --last` deliberately — two limits on "how much output do we hold"
/// would be two numbers to reason about and one of them would be wrong.
pub use oslo_base::capture::MAX as STREAM_MAX;

/// The streams a coordinate can reach.
#[derive(Debug, Default, Clone)]
pub struct Streams {
    /// This pipeline's finished stages, oldest first. Index 0 of a coordinate is the *last* of
    /// these — the stage feeding the command being built.
    stages: Vec<String>,
    /// Previous prompts, newest first, so `-1` is `prompts[0]`.
    prompts: Vec<String>,
}

impl Streams {
    /// Note what a pipeline stage printed.
    pub fn push_stage(&mut self, text: impl Into<String>) {
        self.stages.push(cap(text.into()));
    }

    /// Note what a whole command printed, and start a fresh pipeline.
    ///
    /// The stages are cleared because they belonged to the pipeline that just ended: a coordinate
    /// in the *next* command counting forward from zero would otherwise reach into a pipeline that
    /// is over, which is a different stream than the one it names.
    pub fn push_prompt(&mut self, text: impl Into<String>) {
        self.stages.clear();
        self.prompts.insert(0, cap(text.into()));
        self.prompts.truncate(PROMPTS_KEPT);
    }

    /// Start a new pipeline without recording anything — a command that produced nothing worth
    /// keeping, or one whose output was never captured.
    pub fn end_pipeline(&mut self) {
        self.stages.clear();
    }

    /// The text a coordinate's stream dimension names, if there is one.
    ///
    /// `None` where nothing was captured, which reads as an empty selection rather than an error.
    pub fn text(&self, coord: &Coord) -> Option<&str> {
        let at = match coord.stream {
            coords::Sel::At(at) => at,
            // A range of *streams* is not meaningful — `{0..2:0:0}` would mean "the same line of
            // three different commands", which is a question nobody asks and a syntax nobody would
            // reach for by accident. The first is taken, so the coordinate still reads.
            coords::Sel::Span { from, .. } => from.unwrap_or(0),
        };
        match at >= 0 {
            // Counting back from the newest stage: 0 is the one that just finished.
            true => {
                let back = at as usize;
                self.stages
                    .len()
                    .checked_sub(back + 1)
                    .map(|i| &self.stages[i][..])
            }
            // Previous prompts, newest first.
            false => self.prompts.get((-at - 1) as usize).map(String::as_str),
        }
    }
}

/// Keep the head, not the tail: a coordinate counts from the start, and `{-1}` on a truncated
/// stream is honestly the last line *of what was kept*.
fn cap(mut text: String) -> String {
    if text.len() > STREAM_MAX {
        text.truncate(STREAM_MAX);
    }
    text
}

/// Whether a word contains anything a coordinate could claim.
///
/// A cheap scan, because it runs on every word of every command. It only has to be right about
/// "there is a `{` with a digit, `-`, `*` or `:` after it" — [`substitute`] is what decides.
pub fn looks_like_a_coordinate(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.iter().enumerate().any(|(i, b)| {
        *b == b'{'
            && bytes
                .get(i + 1)
                .is_some_and(|n| n.is_ascii_digit() || matches!(n, b'-' | b'*' | b':' | b'.'))
    })
}

/// Replace the coordinates in one word, answering the words it becomes.
///
/// One word can become several: `{*:0}` on three lines is three arguments, the way `"$@"` is. A
/// word that is *only* a coordinate expands to one argument per value; a coordinate with text
/// around it joins its values with a space, because `pre{*:0}post` has to stay one word to mean
/// anything.
pub fn substitute(text: &str, streams: &Streams) -> Option<Vec<Vec<Run>>> {
    let (before, coord, inside, after) = split(text)?;
    let values = match streams.text(&coord) {
        Some(stream) => coords::select(&coord, stream),
        // Nothing captured: the coordinate reads empty rather than refusing to run, which is the
        // rule everywhere else in this feature.
        None => Vec::new(),
    };
    let _ = inside;

    // A bare coordinate becomes one argument per value.
    if before.is_empty() && after.is_empty() {
        return Some(
            values
                .into_iter()
                .map(|value| vec![Run::new(value, Origin::Quoted)])
                .collect(),
        );
    }
    // Anything else is one word, with the values joined.
    let mut runs = Vec::new();
    if !before.is_empty() {
        runs.push(Run::new(before, Origin::Literal));
    }
    runs.push(Run::new(values.join(" "), Origin::Quoted));
    if !after.is_empty() {
        runs.push(Run::new(after, Origin::Literal));
    }
    Some(vec![runs])
}

/// Split a word around its first coordinate: `(before, coord, inside, after)`.
fn split(text: &str) -> Option<(&str, Coord, &str, &str)> {
    let open = text.find('{')?;
    let close = open + text[open..].find('}')?;
    let inside = &text[open + 1..close];
    let coord = coords::parse(inside)?;
    Some((&text[..open], coord, inside, &text[close + 1..]))
}

#[cfg(test)]
#[path = "streams/tests.rs"]
mod tests;

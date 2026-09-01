//! [`super::diag`] with the drawing taken out, for a build without the `diagnostics` feature.
//!
//! **Every signature is the real one's**, so no call site carries a `cfg` of its own — the one
//! `#[cfg]` in the whole change is in `lib.rs`, choosing between this file and that one. A caller
//! writes the same three lines either way and gets `false` here, which is the answer that means
//! "print your one-liner" and is also the answer the real module gives on a pipe.
//!
//! The arithmetic is kept rather than stubbed: [`floor_boundary`] is a fact about UTF-8, not about
//! rendering, and a caller that uses it to slice its own text needs it to be true in every build.

use std::ops::Range;

/// The real one's, field for field, so a caller can construct one unconditionally.
pub struct Report<'a> {
    pub message: &'a str,
    pub source: &'a str,
    pub label: &'a str,
    pub help: Option<&'a str>,
}

/// Never. Which is what stops the caller building a snapshot it would not use.
pub fn enabled() -> bool {
    false
}

/// The greatest character boundary at or below `at`. Real in every build — see the module note.
pub fn floor_boundary(text: &str, at: usize) -> usize {
    let mut at = at.min(text.len());
    while at > 0 && !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// The words of a command, kept so the queries still answer — a caller may ask which word is at
/// fault for reasons other than drawing it.
pub struct Snapshot {
    spans: Vec<Range<usize>>,
    text: String,
}

impl Snapshot {
    pub fn of<S: AsRef<str>>(words: &[S]) -> Snapshot {
        let mut text = String::new();
        let mut spans = Vec::with_capacity(words.len());
        for word in words {
            if !text.is_empty() {
                text.push(' ');
            }
            let start = text.len();
            text.push_str(word.as_ref());
            spans.push(start..text.len());
        }
        Snapshot { text, spans }
    }

    pub fn index_of(&self, word: &str) -> Option<usize> {
        self.spans
            .iter()
            .position(|span| self.text.get(span.clone()) == Some(word))
    }

    pub fn index_of_positional(&self, n: usize) -> Option<usize> {
        self.spans.get(n).map(|_| n)
    }

    pub fn len(&self) -> usize {
        self.spans.len()
    }

    pub fn is_empty(&self) -> bool {
        self.spans.is_empty()
    }

    /// Nothing is drawn, so the caller prints its one-liner.
    pub fn draw(&self, _at: usize, _report: &Report) -> bool {
        false
    }

    pub fn draw_within(&self, _at: usize, _inside: Range<usize>, _report: &Report) -> bool {
        false
    }
}

pub fn draw_source(_text: &str, _span: Range<usize>, _report: &Report) -> bool {
    false
}

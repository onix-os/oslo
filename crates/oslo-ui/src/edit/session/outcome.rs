//! How reading a line ended.
//!
//! The sibling of [`super::Step`], one level up: a `Step` is what one key resolved to, and this is
//! what the whole read resolved to. Split from [`super`] when that file reached the 600-line limit,
//! along the seam it already had — the loop, the keys it decides, and the answer it returns.

/// How reading a line ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Line(String),
    /// The language toggle was pressed. The read loop switches and reopens with the same text, so
    /// the line and the cursor survive the switch — which is the whole point of a toggle that
    /// works mid-line.
    ToggleLanguage {
        text: String,
        cursor: usize,
    },
    /// Ctrl-C: this line is abandoned, the shell carries on.
    Interrupted,
    /// Ctrl-D on an empty line, or the input ended.
    Eof,
}

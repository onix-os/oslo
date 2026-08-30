//! Streaming a delimited document — `from csv`, `from tsv`.
//!
//! # Two things separate it from `lines`
//!
//! **A record may span lines.** A quoted field is allowed to contain a newline, so cutting a batch
//! at the last `\n` can cut through the middle of one — turning a single record into two, silently,
//! and only for data that happens to quote a newline. A batch therefore ends at the last newline
//! that leaves every quoted field closed, which [`crate::data::tools::formats::is_complete`] answers
//! by running the real parser rather than a second copy of its rules.
//!
//! **The first record is the header**, and it is what names the columns. Every later batch arrives
//! without it and would otherwise take its own first record as the names — so the header is
//! remembered and put back in front of each batch. The parser is then the same one the materialised
//! path uses, on text of the same shape, which is what stops the two answering differently.
//!
//! # What it costs
//!
//! The header is re-parsed once per batch, and each batch is parsed twice — once to ask whether it
//! is complete, once for its rows. Both are bounded by the batch, which is 64 KiB, and neither grows
//! with the document. That is the whole trade: `from json` cannot be streamed at any price, because
//! it has nothing to answer until the closing brace.

use crate::data::tools::formats;

/// What the bridge has to remember between batches.
#[derive(Default)]
pub(super) struct Header {
    /// The first record's text, including its newline. `None` until the first batch has arrived.
    text: Option<String>,
}

impl Header {
    /// The text to hand the parser for this batch: the batch itself the first time, and the header
    /// followed by the batch every time after.
    pub(super) fn before(&mut self, batch: &str, delimiter: char) -> String {
        match &self.text {
            Some(header) => format!("{header}{batch}"),
            None => {
                self.text = first_record(batch, delimiter).map(str::to_string);
                batch.to_string()
            }
        }
    }
}

/// The first record of `text`, including its terminating newline.
///
/// The first newline that leaves the quotes balanced — not simply the first newline, because the
/// header itself may quote one.
fn first_record(text: &str, delimiter: char) -> Option<&str> {
    text.match_indices('\n')
        .map(|(at, _)| at + 1)
        .find(|end| formats::is_complete(&text[..*end], delimiter))
        .map(|end| &text[..end])
}

/// How much of `pending` is whole records, or `None` when none of it is yet.
///
/// **A partial record waits rather than being cut.** When the last newline still leaves a quoted
/// field open, this answers `None` and the reader goes back for more; the field closes on a later
/// read and the batch is taken then. The only input that never yields is one whose quoted field is
/// larger than the whole stream, which the 256 MiB bound already catches.
pub(super) fn whole_records(pending: &str, delimiter: char) -> Option<usize> {
    let end = pending.rfind('\n').map(|at| at + 1)?;
    formats::is_complete(&pending[..end], delimiter).then_some(end)
}

#[cfg(test)]
#[path = "delimited/tests.rs"]
mod tests;

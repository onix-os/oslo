//! What a recorded line *did*, joined to the log row by its id.
//!
//! [`super::log`] keeps the line, the language and when it was typed. It cannot keep the rest —
//! the line is appended **before** it runs, so the directory, the status and the duration are not
//! known yet, and a long command must be in another terminal's history while it is still going.
//!
//! So the outcome is a second row against the same id, written at the command boundary once
//! everything is known, inside the transaction that already updates the aggregate.
//!
//! # Segments
//!
//! Row `0` is the line itself. Rows `1..` are the links of `a && b || c`, in order, each with how
//! it was joined and what it did — including the ones that **never ran**, which is the distinction
//! `exec::pipeline::segments` exists for and the one nothing else in a shell records.
//!
//! # The shape is not stored
//!
//! Only the outcome is. The line is already in the log and `parse_bash_script` is in this same
//! binary, so anything that wants the chain's *structure* re-derives it by parsing rather than
//! reading a second copy that can disagree with the first.
//!
//! # Keyed to be trimmed together
//!
//! The same descending `u64::MAX - id` encoding the log uses, with the segment index after it. One
//! threshold therefore bounds both buckets, and an id the log has dropped cannot leave rows here
//! that nothing will ever join to.

use super::kv::{Fields, Key, Span, Tree, Walk};

/// What a line, or one link of it, did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    /// `0` for the line as a whole; `1..` for its links, in order.
    pub segment: u32,
    /// How this link was joined to the one before it: `&&`, `||`, `;`, or empty for the first.
    pub join: String,
    /// The link as text. Empty for segment `0`, whose text is the log row's own line.
    pub text: String,
    /// `None` when the link never ran, which is neither success nor failure.
    pub status: Option<i32>,
    pub duration_ms: i64,
    /// The directory it ran in, on segment `0` only.
    pub dir_id: u64,
}

impl Outcome {
    /// The line as a whole: where it ran, what it reported, how long it took.
    pub fn line(dir_id: u64, status: Option<i32>, duration_ms: i64) -> Outcome {
        Outcome {
            segment: 0,
            join: String::new(),
            text: String::new(),
            status,
            duration_ms,
            dir_id,
        }
    }

    /// Whether this link actually ran.
    pub fn ran(&self) -> bool {
        self.status.is_some()
    }
}

/// `None` is stored as this, because the encoding carries integers rather than options.
///
/// A status a process can really exit with would be ambiguous; nothing exits `-1` on Linux, where
/// the wait status is eight bits and a signal becomes `128 + n`.
const NEVER_RAN: i64 = -1;

/// The key: the log row's id descending, then the segment index.
fn slot(history_id: u64, segment: u32) -> Vec<u8> {
    Key::with_capacity(16)
        .int(u64::MAX - history_id)
        .int(u64::from(segment))
        .done()
}

/// The span covering every segment of one log row.
fn span_of(history_id: u64) -> Span {
    Span::prefix(Key::with_capacity(8).int(u64::MAX - history_id).done())
}

fn encode(outcome: &Outcome) -> Vec<u8> {
    Key::with_capacity(outcome.text.len() + 40)
        .text(&outcome.join)
        .text(&outcome.text)
        .signed(outcome.status.map_or(NEVER_RAN, i64::from))
        .signed(outcome.duration_ms)
        .int(outcome.dir_id)
        .done()
}

fn decode(key: &[u8], value: &[u8]) -> Option<Outcome> {
    let mut fields = Fields::of(key);
    let _slot = fields.int()?;
    let segment = fields.int()? as u32;

    let mut fields = Fields::of(value);
    let join = fields.text()?.into_owned();
    let text = fields.text()?.into_owned();
    let status = match fields.signed()? {
        NEVER_RAN => None,
        code => Some(code as i32),
    };
    let duration_ms = fields.signed()?;
    let dir_id = fields.int().unwrap_or(0);
    Some(Outcome {
        segment,
        join,
        text,
        status,
        duration_ms,
        dir_id,
    })
}

impl super::Track {
    /// Write what the line at `history_id` did, and what each of its links did.
    ///
    /// One transaction for the lot: a crash must not leave a line recorded as having links it has
    /// no outcome for, nor the reverse.
    pub fn record_outcome(&self, history_id: u64, rows: &[Outcome]) -> bool {
        if rows.is_empty() {
            return true;
        }
        self.store
            .write(|writer| {
                for row in rows {
                    writer.put(Tree::Outcome, slot(history_id, row.segment), encode(row))?;
                }
                Some(())
            })
            .is_some()
    }

    /// Everything recorded about the line at `history_id`, segment order.
    pub fn outcome_of(&self, history_id: u64) -> Vec<Outcome> {
        self.store
            .read(|reader| {
                Some(
                    reader.collect(Tree::Outcome, &span_of(history_id), |key, value| {
                        decode(key, value)
                    }),
                )
            })
            .unwrap_or_default()
    }

    /// Drop outcomes for every log row older than the newest `max`.
    ///
    /// Called from the log's own trim, with the same bound, so the two buckets cannot drift: an
    /// outcome whose line is gone is a row nothing can ever join to.
    pub(super) fn trim_outcomes(&self, max: usize) -> bool {
        let Some(first_doomed) = self.store.read(|reader| {
            let mut kept = 0;
            let mut first_doomed = None;
            reader.scan(Tree::History, &Span::all(), |key, _| {
                if kept >= max {
                    // The history key is the id descending; an outcome key is that plus a segment,
                    // so this is exactly the prefix everything older starts with.
                    first_doomed = Some(key.to_vec());
                    return Walk::Stop;
                }
                kept += 1;
                Walk::On
            });
            Some(first_doomed)
        }) else {
            return false;
        };
        match first_doomed {
            None => true,
            Some(from) => {
                self.store
                    .delete_span_in_chunks(Tree::Outcome, &Span::from(from));
                true
            }
        }
    }
}

#[cfg(test)]
mod tests;

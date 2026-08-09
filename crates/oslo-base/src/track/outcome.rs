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
pub(super) fn slot(history_id: u64, segment: u32) -> Vec<u8> {
    Key::with_capacity(16)
        .int(u64::MAX - history_id)
        .int(u64::from(segment))
        .done()
}

/// The span covering every segment of one log row.
pub(super) fn span_of(history_id: u64) -> Span {
    Span::prefix(Key::with_capacity(8).int(u64::MAX - history_id).done())
}

pub(super) fn encode(outcome: &Outcome) -> Vec<u8> {
    Key::with_capacity(outcome.text.len() + 40)
        .text(&outcome.join)
        .text(&outcome.text)
        .signed(outcome.status.map_or(NEVER_RAN, i64::from))
        .signed(outcome.duration_ms)
        .int(outcome.dir_id)
        .done()
}

pub(super) fn decode(key: &[u8], value: &[u8]) -> Option<Outcome> {
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

/// One line that ran, with everything known about it. The shape a predictor replays.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observation {
    /// The log row's id. Global and ascending, so this *is* the chronological order.
    pub id: u64,
    pub line: String,
    pub mode: String,
    /// Which shell, and where in that shell's run. A gap in `seq` is a command deliberately not
    /// recorded — see [`super::log::Entry::seq`].
    pub session: u32,
    pub seq: u32,
    /// Whether a rule rewrote the line. `false` means this is what was typed.
    pub rewritten: bool,
    /// Whether this line's *arguments* are ones the store would refuse to keep.
    ///
    /// # The trap this exists to close
    ///
    /// `Tree::Run` stores `redact::prepare(line)`, which for a risky line keeps only the head —
    /// `AWS_SECRET=… aws s3 cp …` is remembered as `aws`. The **log** stores the raw line, because
    /// history has to be recallable verbatim or it is not history.
    ///
    /// That is a deliberate split, and it means anything learning from the log learns from text
    /// the aggregate deliberately drops. The difference matters: in the log those arguments are
    /// *recallable if you go looking*; in a predictor they become **offered unprompted**.
    ///
    /// So: learn on [`Self::head`] when this is true, and only ever offer [`Self::line`] when it
    /// is false. Computed at read time from the line, so it costs nothing stored and cannot go
    /// stale against the rules that decide it.
    pub risky: bool,
    /// The command this line runs, with wrappers and assignments stripped — `cargo` for
    /// `time sudo cargo build`. What [`Self::line`] degrades to when [`Self::risky`] is set.
    pub head: String,
    pub dir_id: u64,
    /// `None` when the line never finished: still running, or the shell died. **Not trainable** —
    /// and skipping it should break the sequence, not splice its neighbours together.
    pub status: Option<i32>,
    pub duration_ms: i64,
    /// The links of `a && b || c`, in order. Empty for a line that was not a chain.
    pub segments: Vec<Outcome>,
}

/// A directory an observation points at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    pub id: u64,
    pub path: String,
    /// The worktree it belongs to, when it is in one.
    pub root: Option<String>,
}

impl super::Track {
    /// The newest `limit` lines, oldest first, joined to what each of them did.
    ///
    /// One transaction covers all three buckets so the log, outcomes and directories come from the
    /// same snapshot.
    ///
    /// **The chain's *shape* is not stored and is not returned.** The line is here and
    /// `parse_bash_script` is in this same binary, so a caller that wants the structure re-derives
    /// it rather than reading a second copy that can disagree with the first. What is returned is
    /// the part that cannot be re-derived: what each link actually did.
    pub fn observations(&self, limit: usize) -> (Vec<Observation>, Vec<Place>) {
        if limit == 0 {
            return (Vec::new(), Vec::new());
        }
        self.store
            .read(|reader| {
                let mut newest: Vec<Observation> = Vec::with_capacity(limit.min(1024));
                reader.scan(Tree::History, &Span::all(), |key, value| {
                    if let Some(id) = super::log::id_of_key(key)
                        && let Some(entry) = super::log::entry_of(value)
                    {
                        newest.push(Observation {
                            id,
                            risky: super::redact::is_risky(&entry.line),
                            head: super::redact::head_of(&entry.line),
                            line: entry.line,
                            mode: entry.mode,
                            session: entry.session,
                            seq: entry.seq,
                            rewritten: entry.rewritten,
                            dir_id: 0,
                            status: None,
                            duration_ms: 0,
                            segments: Vec::new(),
                        });
                    }
                    if newest.len() >= limit {
                        Walk::Stop
                    } else {
                        Walk::On
                    }
                });
                // The outcomes for exactly those rows. `Tree::Outcome` shares the log's descending
                // key, so walking it in step with the ids above needs no second sort.
                for observation in &mut newest {
                    for row in
                        reader.collect(Tree::Outcome, &span_of(observation.id), |key, value| {
                            decode(key, value)
                        })
                    {
                        if row.segment == 0 {
                            observation.dir_id = row.dir_id;
                            observation.status = row.status;
                            observation.duration_ms = row.duration_ms;
                        } else {
                            observation.segments.push(row);
                        }
                    }
                }
                newest.reverse();
                let places = reader.collect(Tree::Dir, &Span::all(), |key, value| {
                    let mut fields = Fields::of(key);
                    let id = fields.int()?;
                    let row = super::row::DirRow::decode(value)?;
                    Some(Place {
                        id,
                        path: row.path,
                        root: row.root,
                    })
                });
                Some((newest, places))
            })
            .unwrap_or_default()
    }

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
                super::sync::complete_local(writer, history_id, rows)
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

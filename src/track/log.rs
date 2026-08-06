//! The command log: one row per line typed, in the order it was typed.
//!
//! Lives in the **same store** as the aggregate, in its own [`Tree::History`] bucket. It used to be
//! a second file — `history.db` beside `track.kv` — because it used to be a second *engine*,
//! SQLite where the aggregate was jammdb. Both have been jammdb for a while, so the split was
//! paying for nothing: two opens, two file handles, two floors on disk, and two commits per
//! command with no atomicity between them. A crash between the two left a line in your history
//! that never happened for ranking, or the reverse.
//!
//! # Why a log at all, beside the aggregate
//!
//! They answer different questions. The aggregate folds by `(directory, mode, argv)` and so knows
//! *how often* and *how recently* — which is what ranking needs, and what makes repeats free. The
//! log keeps the order individual executions happened in, which is what `!-2` and `history` need
//! and what folding necessarily throws away.

use super::kv::{Fields, Key, Span, Tree, Walk};
use std::sync::atomic::Ordering;

pub const MODE_SHELL: &str = "sh";
pub const MODE_LUA: &str = "lua";

/// One line, as it was typed and in the language it was typed in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub line: String,
    pub mode: String,
}

/// How many appends go by between trims. See [`super::Track::trim_soon`].
const TRIM_EVERY: usize = 100;

/// The id the first line of a fresh history gets.
///
/// One rather than zero for no deep reason beyond the numbering the `history` builtin prints, which
/// starts at 1.
const FIRST_ID: u64 = 1;

/// Where history is kept, given the environment.
/// The key a line with this id is stored under: the id descending, so that the newest sorts first.
///
/// See the module note. `u64::MAX - id` rather than a reversed comparison because the store
/// compares bytes and has no idea what a key means.
fn slot(id: u64) -> Vec<u8> {
    Key::with_capacity(8).int(u64::MAX - id).done()
}

/// The id a key names, or `None` for bytes this module did not write.
fn id_of(key: &[u8]) -> Option<u64> {
    let mut fields = Fields::of(key);
    let slot = fields.int()?;
    fields.is_empty().then(|| u64::MAX - slot)
}

/// The three fields of a row: the line as typed, its language, and when it was typed.
fn encode(line: &str, mode: &str, at: u64) -> Vec<u8> {
    Key::with_capacity(line.len() + mode.len() + 12)
        .text(line)
        .text(mode)
        .int(at)
        .done()
}

/// A row read back, for another module in this crate that has bytes and wants the fields.
///
/// `Track::forget` matches on the line and the language, which live in the value rather than in
/// the key — so it needs this module.'s encoding rather than a second copy of it.
pub(super) fn entry_of(value: &[u8]) -> Option<Entry> {
    decode(value)
}

/// A row read back, or `None` if it is not one — a truncated value costs one recalled line rather
/// than the whole history.
fn decode(value: &[u8]) -> Option<Entry> {
    let mut fields = Fields::of(value);
    let line = fields.text()?.into_owned();
    let mode = fields.text()?.into_owned();
    Some(Entry { line, mode })
}

/// Seconds since the epoch, or zero on a clock that is before it.
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

/// The log's half of the store.
impl super::Track {
    /// Remember one line.
    pub fn append(&self, line: &str, mode: &str) -> bool {
        let at = now();
        self.store
            .write(|writer| {
                let id = next_id(writer);
                writer.put(Tree::History, slot(id), encode(line, mode, at))
            })
            .is_some()
    }

    /// The most recent `limit` lines, oldest first — the order a line editor wants them in.
    ///
    /// A walk of `limit` rows from the start of the bucket and no further, because the bucket is
    /// keyed newest-first. The reversal at the end is of what was taken, not of the history.
    pub fn recent(&self, limit: usize) -> Vec<Entry> {
        if limit == 0 {
            return Vec::new();
        }
        self.store
            .read(|reader| {
                let mut newest = Vec::with_capacity(limit.min(1024));
                reader.scan(Tree::History, &Span::all(), |_, value| {
                    if let Some(entry) = decode(value) {
                        newest.push(entry);
                    }
                    if newest.len() >= limit {
                        Walk::Stop
                    } else {
                        Walk::On
                    }
                });
                newest.reverse();
                Some(newest)
            })
            .unwrap_or_default()
    }

    /// Drop everything. `history -c`.
    ///
    /// Answers `true` for an empty history as the `DELETE FROM history` it replaces did: there was
    /// nothing to clear is not a failure to clear it.
    pub fn clear(&self) -> bool {
        self.store
            .write(|writer| {
                writer.clear(Tree::History);
                Some(())
            })
            .is_some()
    }

    /// Trim to the newest `max` lines, which is what `$HISTSIZE` asks for.
    ///
    /// The rows to keep are the first `max` of the bucket, so everything past them is one span with
    /// no upper end — the whole of the trim is naming the key where that span starts. Nothing is
    /// read beyond it.
    ///
    /// The deleting is `Store::delete_span_in_chunks` and not the one-transaction version, which
    /// is not a preference. A single transaction that deletes a hundred rows from a bucket of a few
    /// thousand panics inside jammdb and deletes *nothing*; the seam has the measurements. A
    /// history at the default `HISTSIZE` of ten thousand is exactly that shape, every hundred lines
    /// typed, for the rest of the machine's life — so this is the difference between a bound and
    /// the appearance of one.
    pub fn trim(&self, max: usize) -> bool {
        let Some(first_doomed) = self.store.read(|reader| {
            let mut kept = 0;
            let mut first_doomed = None;
            reader.scan(Tree::History, &Span::all(), |key, _| {
                if kept >= max {
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
            // Already inside the bound, which is nearly every call: nothing is written at all.
            None => true,
            Some(from) => {
                self.store
                    .delete_span_in_chunks(Tree::History, &Span::from(from))
                    > 0
            }
        }
    }

    /// Trim, but not more often than one line in `TRIM_EVERY`.
    ///
    /// The REPL used to call [`super::Track::trim`] after every single command, and under SQL `trim` was
    /// a `DELETE ... WHERE id NOT IN (SELECT ... LIMIT N)` — a full scan of the table, per line
    /// typed, to delete nothing at all in the overwhelming majority of cases. The scan is gone with
    /// the SQL, but the batching stays and the reason is now the other one: a trim is a *write*,
    /// and a write takes the file's exclusive lock, so a trim per line is a lock per line that
    /// every other terminal's next keystroke queues behind. A hundred lines of slack against a
    /// ten-thousand-line bound is not a bound anybody can perceive.
    ///
    /// The loop trims unconditionally on the way out, so a short session still ends bounded.
    pub fn trim_soon(&self, max: usize) {
        if self.since_trim.fetch_add(1, Ordering::Relaxed) + 1 >= TRIM_EVERY {
            self.since_trim.store(0, Ordering::Relaxed);
            self.trim(max);
        }
    }
}

/// The id the next line gets: one above the newest there is.
///
/// One row read, because the newest is the first row. Taken inside the same write transaction as
/// the `put` that uses it, which is what stops two terminals appending under one id.
fn next_id(reader: &super::kv::Reader<'_, '_>) -> u64 {
    reader
        .find(Tree::History, &Span::all(), |key, _| id_of(key))
        .map_or(FIRST_ID, |newest| newest.saturating_add(1))
}

#[cfg(test)]
mod tests;

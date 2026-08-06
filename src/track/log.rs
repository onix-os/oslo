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
    /// Which shell typed it. `0` for a row written before sessions were recorded.
    ///
    /// **Not the pid.** A small ordinal from [`Tree::Meta`], four bytes rather than the eighteen a
    /// `pid-starttime` string costs on every row, and the only question anything asks of it is
    /// "same shell or not".
    pub session: u32,
    /// Position within that session, from one. `0` for a row written before this existed.
    ///
    /// **Not redundant with the row's id**, which is global and ascending. A secret command is
    /// never appended and so consumes no id — the global sequence has no gap to show for it, and
    /// anything replaying the log would splice the neighbours together and learn a transition that
    /// never happened. A per-session counter that *does* skip is what makes the omission visible.
    pub seq: u32,
    /// Whether what is stored is what was typed.
    ///
    /// Set when a rule rewrote the line before it was recorded. A declared transformation is data;
    /// a silent omission is a hole, and this is the difference between them.
    pub rewritten: bool,
}

impl Entry {
    /// A line as typed. `session` and `seq` are filled in by [`super::Track::append_entry`].
    pub fn new(line: &str, mode: &str) -> Entry {
        Entry {
            line: line.to_string(),
            mode: mode.to_string(),
            session: 0,
            seq: 0,
            rewritten: false,
        }
    }
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
/// The id a key names, for the join in `super::outcome`.
pub(super) fn id_of_key(key: &[u8]) -> Option<u64> {
    id_of(key)
}

fn id_of(key: &[u8]) -> Option<u64> {
    let mut fields = Fields::of(key);
    let slot = fields.int()?;
    fields.is_empty().then(|| u64::MAX - slot)
}

/// A row: the line as typed, its language, when it was typed, and who by.
///
/// **Appended to, never reordered.** `decode` reads the first two fields and treats everything
/// after them as optional, so a row written by an older oslo reads back with the new fields at
/// their defaults and a row written by a newer one is still readable here. That is what makes
/// adding a field a non-event rather than a migration.
fn encode(entry: &Entry, at: u64) -> Vec<u8> {
    Key::with_capacity(entry.line.len() + entry.mode.len() + 28)
        .text(&entry.line)
        .text(&entry.mode)
        .int(at)
        .int(u64::from(entry.session))
        .int(u64::from(entry.seq))
        .int(u64::from(entry.rewritten))
        .done()
}

/// A row read back, for another module in this crate that has bytes and wants the fields.
///
/// `Track::forget` matches on the line and the language, which live in the value rather than in
/// the key — so it needs this module's encoding rather than a second copy of it.
pub(super) fn entry_of(value: &[u8]) -> Option<Entry> {
    decode(value)
}

/// A row read back, or `None` if it is not one — a truncated value costs one recalled line rather
/// than the whole history.
fn decode(value: &[u8]) -> Option<Entry> {
    let mut fields = Fields::of(value);
    let line = fields.text()?.into_owned();
    let mode = fields.text()?.into_owned();
    // Optional from here: a row from before these fields existed simply has none of them, and
    // reads back as a session-less line rather than as a corrupt one.
    let _at = fields.int();
    let session = fields.int().unwrap_or(0) as u32;
    let seq = fields.int().unwrap_or(0) as u32;
    let rewritten = fields.int().unwrap_or(0) != 0;
    Some(Entry {
        line,
        mode,
        session,
        seq,
        rewritten,
    })
}

/// Seconds since the epoch, or zero on a clock that is before it.
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

/// `Tree::Meta` key holding the last session ordinal handed out.
const SESSION_COUNTER: &str = "session";

/// This shell's ordinal, allocated once and kept.
///
/// Taken on the first line rather than at startup, so a terminal that is opened and closed without
/// running anything neither consumes an ordinal nor costs a write.
static SESSION: std::sync::OnceLock<u32> = std::sync::OnceLock::new();

/// How many lines this shell has appended — the `seq` on the row. See [`Entry::seq`].
static TYPED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// The log's half of the store.
impl super::Track {
    /// Remember one line, and answer the id it went in under.
    ///
    /// The id is what an outcome joins on, so a caller that wants to record what the line *did*
    /// keeps it. `None` when the store would not take the row — the same "a shell whose tracker
    /// will not open is a working shell" rule the rest of this module follows.
    pub fn append(&self, line: &str, mode: &str) -> Option<u64> {
        self.append_entry(&Entry::new(line, mode))
    }

    /// [`Self::append`] for a caller with more to say than the line — a rewritten row, say.
    ///
    /// `session` and `seq` are filled in here rather than by the caller: they are facts about this
    /// shell, and a caller that had to supply them could get them wrong.
    pub fn append_entry(&self, entry: &Entry) -> Option<u64> {
        let at = now();
        let mut row = entry.clone();
        row.session = self.session_ordinal();
        row.seq = TYPED.fetch_add(1, Ordering::Relaxed) + 1;
        self.store.write(|writer| {
            let id = next_id(writer);
            writer
                .put(Tree::History, slot(id), encode(&row, at))
                .map(|()| id)
        })
    }

    /// Replace what the row at `history_id` says was typed, and mark it as rewritten.
    ///
    /// **In place, keeping the id**, because the id is what the outcome rows join on and what the
    /// editor's own copy is ordered by. Deleting and re-appending would renumber the line and
    /// orphan everything already written against it.
    ///
    /// The `rewritten` flag is the whole point: a reader can then tell "this is what was typed"
    /// from "this is what a rule kept", which is the difference between a transformation and a
    /// hole. See [`Entry::rewritten`].
    pub fn rewrite_line(&self, history_id: u64, line: &str) -> bool {
        self.store
            .write(|writer| {
                let key = slot(history_id);
                let mut row = decode(&writer.get(Tree::History, &key)?)?;
                if row.line == line {
                    return Some(());
                }
                row.line = line.to_string();
                row.rewritten = true;
                writer.put(Tree::History, key, encode(&row, now()))
            })
            .is_some()
    }

    /// Take the row at `history_id` out of the log entirely, outcomes and all.
    ///
    /// What a `pre-record` rule returning `false` asks for. The outcomes go with it for the same
    /// reason the trim takes them: a row nothing can join to is a row nothing will ever read.
    pub fn drop_line(&self, history_id: u64) -> bool {
        self.store
            .write(|writer| {
                writer.delete(Tree::History, &slot(history_id));
                Some(())
            })
            .is_some()
            && self.drop_outcome(history_id)
    }

    /// This shell's session ordinal, allocated from `Tree::Meta` on first use.
    ///
    /// Zero when the store will not answer, which `decode` already reads as "no session recorded"
    /// rather than as membership of a session numbered zero.
    fn session_ordinal(&self) -> u32 {
        *SESSION.get_or_init(|| {
            self.store
                .write(|writer| {
                    let key = Key::with_capacity(16).text(SESSION_COUNTER).done();
                    let last = writer
                        .get(Tree::Meta, &key)
                        .and_then(|bytes| Fields::of(&bytes).int())
                        .unwrap_or(0);
                    let next = last.saturating_add(1);
                    writer
                        .put(Tree::Meta, key, Key::with_capacity(8).int(next).done())
                        .map(|()| next as u32)
                })
                .unwrap_or(0)
        })
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
                // **Before the rows themselves**, because the span is computed from the log's own
                // keys: once the line is gone there is nothing left to say which outcomes belonged
                // to it, and they would sit there for ever in a store with no `VACUUM`.
                self.trim_outcomes(max);
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

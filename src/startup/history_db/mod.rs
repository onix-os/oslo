//! Command history in a database rather than a flat file.
//!
//! A text file cannot answer the question the shell actually has. oslo reads two languages, and a
//! line recalled from history has to run in the one it was typed in — recall a Lua line while the
//! prompt is in shell mode and a flat file gives you no way to know. The mode is a field here, so
//! there is nothing to guess and no marker smuggled into the text.
//!
//! # Where it lives
//!
//! `$XDG_DATA_HOME/oslo/history.db`, falling back to `~/.local/share/oslo/history.db`. History is
//! state the user accumulates, not configuration they wrote, so it belongs under the data
//! directory rather than in `$HOME` or beside the config.
//!
//! # There is no runtime here any more
//!
//! This module used to own a `tokio` current-thread runtime and `block_on` every call, because
//! turso's API is async and oslo's REPL is not. Both are gone: the store underneath is
//! [`oslo::track::kv`], which is `jammdb` behind a seam and is synchronous all the way down. Every
//! call to the engine goes through that seam and **nothing here may `use jammdb`** — read
//! `src/track/kv/mod.rs` before changing anything below, because three of its measured facts are
//! load-bearing for this file:
//!
//! * the store holds no handle, so a second terminal is never blocked waiting on this one;
//! * a file that is not a jammdb database makes `DB::open` *panic* rather than error, which is why
//!   [`History::open`] renames a file it cannot read out of the way before opening;
//! * the file grows in 8 MiB steps and never shrinks, and a large delete is the shape that panics
//!   inside the engine, which together are why [`History::trim`] is written the way it is.
//!
//! That last one is worth the numbers, measured on this file with real lines in it:
//!
//! ```text
//!    400 lines      131,072 B        10,000 lines   8,519,680 B
//!    500 lines      131,072 B        then trimmed
//!  1,000 lines    8,519,680 B          to 500       8,519,680 B
//! ```
//!
//! The cliff is between 500 and 1,000 rows, there is no `VACUUM`, and a trim gives nothing back. So
//! the default `HISTSIZE` of 10,000 costs 8.5 MB of disk for the rest of the machine's life, where
//! turso's file was a few hundred KB — a real regression, and the only lever is `HISTSIZE`. A
//! history in the hundreds stays at 128 KiB.
//!
//! There is also no write-ahead log, so there is nothing to checkpoint. `History::checkpoint` went
//! with turso; `settle_stores` in the REPL now only trims.
//!
//! # The newest line is the first row
//!
//! The only read this file does is "the last N", and the seam's cursor walks *forwards* — a bucket
//! keyed by ascending id would need a reverse cursor it does not have, or a walk of the whole
//! history to reach the end of it. So the key is the id **descending**: `u64::MAX - id`, eight
//! bytes big-endian. The newest line is then the first row in the bucket and "the last N" is a walk
//! of N rows from the start, whatever the history's size. [`History::trim`] falls out of the same
//! ordering: everything to keep is a prefix of the bucket and everything to drop is one contiguous
//! span at the end of it, deleted as a range rather than found by a scan.
//!
//! # A command that spans several lines is one entry
//!
//! `$HISTFILE` is newline-separated and therefore cannot hold a `for` loop as one entry; it is
//! stored here as a field, and a field is framed by length and terminator rather than by a
//! separator anybody has to escape. The value is `(line, mode, at)` in the seam's own encoding,
//! where a newline is an ordinary byte and the only byte with a meaning is `0x00`, which that
//! encoding escapes. So a multi-line command round-trips exactly as typed — see
//! `a_command_typed_across_several_lines_comes_back_as_one_entry`.
//!
//! The timestamp is written and nothing reads it yet, exactly as the `at` column it replaces was
//! written and never selected. It is what a `history -t` would need, and adding it later would need
//! a migration where recording it now needs none.

use oslo::track::kv::{Fields, Key, Span, Store, Tree, Walk};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

/// The language a line was typed in, as stored.
///
/// A string rather than an integer so that a dump of the store is legible without a decoder ring:
/// the two-letter tag is what the mode is called everywhere else in the shell.
pub const MODE_SHELL: &str = "sh";
pub const MODE_LUA: &str = "lua";

/// One line, as it was typed and in the language it was typed in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub line: String,
    pub mode: String,
}

/// How many appends go by between trims. See [`History::trim_soon`].
const TRIM_EVERY: usize = 100;

/// The id the first line of a fresh history gets.
///
/// One rather than zero for no deep reason beyond the numbering the `history` builtin prints, which
/// starts at 1.
const FIRST_ID: u64 = 1;

/// Where history is kept, given the environment.
///
/// `$XDG_DATA_HOME` first, then the specification's own default of `~/.local/share`. Returns
/// `None` when neither is knowable, which is a shell with no home — a container's `nobody`, say —
/// and which must run without a history rather than fail.
pub fn database_path(xdg_data: Option<&str>, home: Option<&str>) -> Option<PathBuf> {
    let base = match xdg_data {
        Some(dir) if !dir.trim().is_empty() => PathBuf::from(dir),
        _ => PathBuf::from(home?).join(".local/share"),
    };
    Some(base.join("oslo/history.db"))
}

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

/// An open history.
pub struct History {
    store: Store,
    /// Appends since the last trim, so the trim is amortised rather than paid per line.
    since_trim: AtomicUsize,
}

impl History {
    /// Open, creating the file and its directory if they are not there.
    ///
    /// Every failure answers `None` rather than propagating: a shell whose history cannot be
    /// opened is a working shell without history, and refusing to start over it would be absurd.
    ///
    /// A file at this path that is not ours — an older build's database, or something a disk
    /// corrupted — is renamed aside rather than opened or deleted. Without that, an unreadable
    /// file means `Store::open` refuses for ever and the shell silently has no history until
    /// somebody deletes it by hand. `rename` within one directory is atomic, so two terminals
    /// starting together cannot both move it: the loser finds nothing at the source and does
    /// nothing, which is the right outcome.
    pub fn open(path: &Path) -> Option<History> {
        if path.is_file() && !oslo::track::kv::is_a_database(path) {
            let _ = std::fs::rename(path, path.with_extension("db.unreadable"));
        }
        Some(History {
            store: Store::open(path)?,
            since_trim: AtomicUsize::new(0),
        })
    }

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
    /// The deleting is [`Store::delete_span_in_chunks`] and not the one-transaction version, which
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

    /// Trim, but not more often than one line in [`TRIM_EVERY`].
    ///
    /// The REPL used to call [`History::trim`] after every single command, and under SQL `trim` was
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
fn next_id(reader: &oslo::track::kv::Reader<'_, '_>) -> u64 {
    reader
        .find(Tree::History, &Span::all(), |key, _| id_of(key))
        .map_or(FIRST_ID, |newest| newest.saturating_add(1))
}

#[cfg(test)]
mod tests;

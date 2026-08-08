//! What keeps the file from being the largest thing in `~/.local/share/oslo`.
//!
//! The store is an aggregate, so repeats — the entire point of the ranking — cost nothing after the
//! first row. What still grows without a bound is the tail of lines run exactly once: a `git commit
//! -m "…"`, a `kill 12345`, a one-off path. Those are not suggestions and never will be, so a row
//! that has been run once and not touched in ninety days goes. Directories that have stopped
//! existing go too, but only after thirty days of not existing: an unmounted USB stick is not a
//! deleted directory, and a store that forgot a project every time a disk was late to mount would
//! be worse than one that forgot nothing.
//!
//! # This file is what bounds the file now, and the number is 8 MiB
//!
//! Under turso the sweep was tidiness with a WAL problem attached. It is not tidiness here.
//! jammdb allocates in one 8 MiB step and never gives it back — measured, 400 rows fit in the
//! 128 KiB a fresh store is born with, and somewhere between 500 and 1,000 rows the file jumps to
//! 8.5 MiB and stays there for good. There is no `VACUUM` and no way to shrink it. So
//! the per-directory cap and the ninety-day rule are the difference between a store that costs 128 KiB
//! and one that costs 8.5 MiB for the rest of the machine's life, and the checkpoint that used to
//! be the headline of this module is gone entirely: there is no write-ahead log, no `-wal`, no
//! `-shm`. One file.
//!
//! # Nothing here holds a transaction open
//!
//! This is the rule the seam asks for and it shapes every function below. The engine takes a
//! **whole-file exclusive lock** for the duration of a transaction, read or write, so a sweep that
//! ran as one transaction would queue every other terminal's next keystroke behind it. So the
//! sweep is a sequence of short transactions: read what has to go, let go of the lock, delete it in
//! chunks, let go again between each.
//!
//! The longest single lock the sweep takes is one pass over every `run` row — the age rule and the
//! cap each need one — which is a few milliseconds against the 25,000 rows the store is designed to
//! reach, once a day. That is the honest cost and it is the one thing here that is not chunked:
//! resuming a scan from its last key would be a bound on a stall nobody is in a position to notice.
//!
//! The `stat()` of every remembered directory is the sharper case, and it happens **outside every
//! transaction**. A `stat` on an unresponsive NFS mount or a sleeping disk can take seconds, and
//! doing it while holding the lock would freeze every prompt on the machine. The SQL version did
//! it inside a live query and got away with it because SQLite's readers do not block writers;
//! this one cannot, and the fix is to stat a list rather than a cursor.
//!
//! # Contract item 4: the cascade is `Track::forget_directory`, and there is nothing else
//!
//! `ON DELETE CASCADE` is gone with the foreign keys. Dropping a directory means, in this order:
//! the `run_argv` twin of every `run` row in the range beginning with its id, then those rows, then
//! its three index entries and the row itself. If a future caller ever wants
//! `oslo.track.forget(path)`, it is that function and nothing more.
//!
//! It is deliberately **not** one transaction — see the note on the function, which is the one
//! place in this module where a jammdb defect rather than a design decision picks the shape.

use super::db::{LAST_PRUNE, Track, now, put_dir, read_dir, set_meta};
use super::kv::{Reader, Span, Tree, Walk};
use super::row::{DirRow, RunRow, field, key, span};
use std::path::Path;

/// How old a `runs = 1` row is allowed to get.
const RUN_MAX_AGE: i64 = 90 * 24 * 60 * 60;

/// How long a directory is allowed to be missing before it is forgotten, rather than merely noted
/// as absent. Long enough that an unplugged disk, a stopped container or an unmounted share is a
/// pause rather than a deletion.
const GONE_MAX_AGE: i64 = 30 * 24 * 60 * 60;

/// How often the sweep is worth running at all.
const SWEEP_EVERY: i64 = 24 * 60 * 60;

/// The most lines one directory may keep.
///
/// The bound that needs no schedule: it protects against one pathological directory — a
/// generated-script farm, a shell loop that types a new line every time — rather than against time.
/// See the module note for why it is now the difference between two file sizes rather than a
/// nicety.
const RUNS_PER_DIR: usize = 500;

/// How many directories one transaction marks as missing before letting go of the file.
///
/// Only the marks, which are `put`s: every *delete* in this module goes through the seam's own
/// budget instead, because a delete large enough to empty a node is thrown away rather than
/// refused. Small enough that the longest anybody waits is a fraction of a millisecond, large
/// enough that an unplugged disk full of remembered directories is a handful of commits rather
/// than one `fsync` per directory.
const CHUNK: usize = 256;

impl Track {
    /// Run the sweep now, whatever the stamp says.
    ///
    /// Answers how many `run` rows it removed, which is only of interest to a test — nothing in the
    /// shell branches on it.
    ///
    /// The order matters and is the SQL's: the age rule first, then the cap, so that the cap is
    /// measured against what is left rather than against rows that were about to go anyway. Each
    /// phase reads under one lock and writes under others, and holds none of them across the
    /// `stat`s.
    pub fn sweep(&self) -> u64 {
        if !self.writable {
            return 0;
        }
        let at = now();
        let stale = self
            .store
            .read(|reader| Some(stale_runs(reader, at - RUN_MAX_AGE)))
            .unwrap_or_default();
        let mut removed = self.drop_runs(&stale);

        let over = self
            .store
            .read(|reader| Some(over_the_cap(reader)))
            .unwrap_or_default();
        removed += self.drop_runs(&over);

        self.forget_vanished(at);
        self.store.write(|writer| set_meta(writer, LAST_PRUNE, at));
        removed
    }

    /// Whether the sweep is due, which is once a day.
    ///
    /// A store that has never been swept is due immediately: the stamp is written by the sweep
    /// itself, so a file created before this code existed is caught on the first shell that opens
    /// it rather than a day later.
    pub fn sweep_is_due(&self) -> bool {
        if !self.writable {
            return false;
        }
        let last = self
            .store
            .read(|reader| super::db::meta(reader, LAST_PRUNE))
            .unwrap_or(0);
        now().saturating_sub(last) >= SWEEP_EVERY
    }

    /// Delete run rows and the `run_argv` entries that name them.
    ///
    /// Two passes rather than one, and the index goes first. The order is what makes an interrupted
    /// delete harmless: an index missing an entry for a row that still exists costs one line its
    /// place in the widened suggestion until the next sweep, where an index entry left pointing at
    /// a row that is gone is bytes nothing will ever collect. The first is recoverable and the
    /// second is not, so the recoverable state is the one a crash is allowed to leave behind.
    fn drop_runs(&self, doomed: &[Vec<u8>]) -> u64 {
        let twins: Vec<Vec<u8>> = doomed
            .iter()
            .filter_map(|run| key::twin_of_run(run))
            .collect();
        self.store.delete_keys_in_chunks(Tree::RunByArgv, &twins);
        self.store.delete_keys_in_chunks(Tree::Run, doomed) as u64
    }

    /// `stat()` every directory, note the ones that are gone, forget the ones that have been gone
    /// long enough.
    ///
    /// Two phases and two rules on purpose. Noting is idempotent, and a directory that came back
    /// and was walked into has had its mark cleared by the arrival — so a disk that is late to
    /// mount costs the store nothing at all.
    fn forget_vanished(&self, at: i64) {
        let known: Vec<(u64, String)> = self
            .store
            .read(|reader| {
                Some(reader.collect(Tree::Dir, &Span::all(), |key, value| {
                    Some((field::leading_id(key)?, DirRow::decode(value)?.path))
                }))
            })
            .unwrap_or_default();

        // The transaction is over. See the module note: a `stat` that blocks must not block a
        // prompt in another terminal, and here it cannot, because nothing is holding the file.
        let gone: Vec<u64> = known
            .into_iter()
            .filter(|(_, path)| !Path::new(path).is_dir())
            .map(|(id, _)| id)
            .collect();
        for chunk in gone.chunks(CHUNK) {
            self.store.write(|writer| {
                for id in chunk {
                    note_missing(writer, *id, at);
                }
                Some(())
            });
        }

        let doomed: Vec<u64> = self
            .store
            .read(|reader| {
                Some(reader.collect(Tree::Dir, &Span::all(), |key, value| {
                    let row = DirRow::decode(value)?;
                    (row.missing_since? < at - GONE_MAX_AGE).then(|| field::leading_id(key))?
                }))
            })
            .unwrap_or_default();
        for id in doomed {
            self.forget_directory(id);
        }
    }

    /// Contract item 4: a directory, everything that was ever run in it, and every index entry
    /// naming either.
    ///
    /// **Not one transaction, and it must not be one.** A directory at the cap is five hundred run
    /// rows and five hundred index entries, and jammdb 0.11 throws away a transaction that deletes
    /// that much in one go — silently, so the cascade would look like it had happened. See
    /// [`super::kv::Store::delete_span_in_chunks`], which is where that was measured.
    ///
    /// What replaces the atomicity is an order in which every intermediate state is one the store
    /// can be left in. The index entries go before the rows they name; the runs go before the
    /// directory that owns them; the directory row goes last. So a crash anywhere leaves a
    /// directory that is still marked missing, still older than the limit, and still forgotten by
    /// the next sweep, which picks up where this one stopped. The one thing that must never happen
    /// — a `run` row surviving the directory it is attributed to — is the one thing this order
    /// makes impossible.
    fn forget_directory(&self, id: u64) -> u64 {
        let runs: Vec<Vec<u8>> = self
            .store
            .read(|reader| {
                Some(reader.collect(Tree::Run, &span::runs_of(id), |key, _| Some(key.to_vec())))
            })
            .unwrap_or_default();
        let twins: Vec<Vec<u8>> = runs
            .iter()
            .filter_map(|run| key::twin_of_run(run))
            .collect();
        self.store.delete_keys_in_chunks(Tree::RunByArgv, &twins);
        let gone = self
            .store
            .delete_span_in_chunks(Tree::Run, &span::runs_of(id)) as u64;

        // Four point deletes, which is nothing against the budget, so the directory and the three
        // indexes that name it go together.
        self.store.write(|writer| {
            if let Some(row) = read_dir(writer, id) {
                writer.delete(Tree::DirByPath, &key::by_path(&row.path));
                writer.delete(Tree::DirByBase, &key::by_base(&row.base, id));
                if let Some(root) = &row.root {
                    writer.delete(Tree::DirByRoot, &key::by_root(root, id));
                }
            }
            writer.delete(Tree::Dir, &key::dir(id));
            Some(())
        });
        gone
    }
}

/// Hand a due sweep to a thread and let the prompt come up.
///
/// The `&'static` is what makes this safe to detach: the store is the process-global installed by
/// the interactive loop and lives as long as the process does, so there is no join to wait on and
/// nothing to outlive. A shell that exits mid-sweep loses a sweep, which the next one redoes — and
/// because the sweep is a sequence of short transactions rather than one, a shell that dies in the
/// middle of it leaves the store consistent at whatever chunk it had reached.
pub fn sweep_soon(track: &'static Track) {
    if !track.sweep_is_due() {
        return;
    }
    std::thread::spawn(move || {
        track.sweep();
    });
}

/// Lines run once and never again, long enough ago to be certain.
///
/// One scan of the whole bucket, allocating only for the rows it is going to delete. SQL had a
/// partial index for this (`run_age ... WHERE runs = 1`); a key-value store has no partial indexes,
/// and an index maintained on every command to save one scan a day would be the wrong trade — it
/// would also be more bytes in a file whose size is the thing this module exists to hold down.
fn stale_runs(reader: &Reader<'_, '_>, cutoff: i64) -> Vec<Vec<u8>> {
    reader.collect(Tree::Run, &Span::all(), |key, value| {
        let row = RunRow::decode(value)?;
        (row.runs == 1 && row.last_at < cutoff).then(|| key.to_vec())
    })
}

/// Everything in one directory beyond the cap: fewest runs first, then oldest.
///
/// The SQL was a correlated subquery — `DELETE FROM run WHERE id IN (SELECT id ... ORDER BY runs
/// ASC, last_at ASC LIMIT MAX(0, COUNT(*) - 500))` — and it has to mean the same thing here. It
/// does: the same two keys, ascending, and the same count. The order that keeps the habits and
/// drops the accidents.
///
/// Two passes rather than one. The first counts, and a `run` key begins with its directory, so a
/// range walks one directory's rows before it reaches the next and the count is a counter rather
/// than a map. The second visits only the directories actually over the cap, which is what SQL's
/// `HAVING COUNT(*) > ?` bought and is nothing at all for the overwhelming majority of stores.
fn over_the_cap(reader: &Reader<'_, '_>) -> Vec<Vec<u8>> {
    let mut over = Vec::new();
    let mut current: Option<u64> = None;
    let mut rows = 0usize;
    reader.scan(Tree::Run, &Span::all(), |key, _| {
        let dir = field::leading_id(key);
        if dir != current {
            if rows > RUNS_PER_DIR
                && let Some(id) = current
            {
                over.push(id);
            }
            current = dir;
            rows = 0;
        }
        rows += 1;
        Walk::On
    });
    if rows > RUNS_PER_DIR
        && let Some(id) = current
    {
        over.push(id);
    }

    let mut doomed = Vec::new();
    for id in over {
        let mut group: Vec<(i64, i64, Vec<u8>)> =
            reader.collect(Tree::Run, &span::runs_of(id), |key, value| {
                let row = RunRow::decode(value)?;
                Some((row.runs, row.last_at, key.to_vec()))
            });
        let Some(excess) = group.len().checked_sub(RUNS_PER_DIR) else {
            continue;
        };
        group.sort_by_key(|a| (a.0, a.1));
        doomed.extend(group.drain(..excess).map(|(_, _, key)| key));
    }
    doomed
}

/// Note a directory as absent, without forgetting it. Idempotent: a mark already there stands, so
/// the thirty days are counted from when it first went rather than from the last sweep.
fn note_missing(writer: &super::kv::Writer<'_, '_>, id: u64, at: i64) {
    let Some(was) = read_dir(writer, id) else {
        return;
    };
    if was.missing_since.is_some() {
        return;
    }
    let mut row = was.clone();
    row.missing_since = Some(at);
    put_dir(writer, id, Some(&was), &row);
}

#[cfg(test)]
mod tests;

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
//! # The WAL is the real reason this file exists
//!
//! Measured on turso 0.7.2: the write-ahead log grows to a couple of megabytes and **never**
//! truncates on its own — not when the `Database` drops, not on the next open. It outgrows the
//! database it belongs to. `PRAGMA wal_checkpoint(TRUNCATE)` takes it to exactly zero and is the
//! only thing that does, so it runs at the end of every sweep and again when the shell exits.
//! Through `query()`, never `execute()`: a pragma answers with a row and `execute` refuses one.
//!
//! # No daemon
//!
//! A `meta.last_prune` stamp, read when the store opens; if it is more than a day old the sweep is
//! handed to a detached thread and the prompt never waits for it. That is not new machinery —
//! `interactive::command_index::warm` already spawns exactly this kind of one-shot startup thread
//! for the `$PATH` scan, and for the same reason: whatever the shell is doing before the first
//! prompt is time the sweep gets for free.
//!
//! Deliberately not zoxide's `_ZO_MAXAGE` global rescale, whose failure is documented in its own
//! issue #292: one burst of new directories pushes the total over the threshold and a single
//! proportional multiply-and-prune wipes the recent long tail while a stale high-rank entry
//! survives untouched. A per-row age rule plus a per-directory cap costs the same and cannot do
//! that.

use super::db::{Track, now, runtime};
use turso::{Connection, Value};

/// How old a `runs = 1` row is allowed to get. The partial index `run_age` makes this a range scan.
const RUN_MAX_AGE: i64 = 90 * 24 * 60 * 60;

/// How long a directory is allowed to be missing before it is forgotten, rather than merely noted
/// as absent. Long enough that an unplugged disk, a stopped container or an unmounted share is a
/// pause rather than a deletion.
const GONE_MAX_AGE: i64 = 30 * 24 * 60 * 60;

/// How often the sweep is worth running at all.
const SWEEP_EVERY: i64 = 24 * 60 * 60;

/// The most lines one directory may keep.
///
/// This is the bound that needs no schedule: it protects against one pathological directory — a
/// generated-script farm, a shell loop that types a new line every time — rather than against time,
/// and so it holds even in a shell that is never open long enough for the daily sweep to fire.
const RUNS_PER_DIR: i64 = 500;

/// The stamp saying when the sweep last ran.
const READ_STAMP: &str = "SELECT value FROM meta WHERE key = 'last_prune'";
const WRITE_STAMP: &str = "INSERT INTO meta (key, value) VALUES ('last_prune', ?1) \
     ON CONFLICT(key) DO UPDATE SET value = excluded.value";

/// Lines run once and never again, long enough ago to be certain.
const OLD_RUNS: &str = "DELETE FROM run WHERE runs = 1 AND last_at < ?1";

/// Note a directory as absent, without forgetting it.
const NOTE_MISSING: &str =
    "UPDATE dir SET missing_since = ?1 WHERE id = ?2 AND missing_since IS NULL";

/// And forget it once it has been absent long enough. `ON DELETE CASCADE` takes its runs with it,
/// which is real here because the connection sets `PRAGMA foreign_keys = ON`.
const DROP_MISSING: &str = "DELETE FROM dir WHERE missing_since IS NOT NULL AND missing_since < ?1";

/// Everything in one directory beyond the cap: fewest runs first, then oldest, which is the order
/// that keeps the habits and drops the accidents.
const OVER_CAP: &str = "DELETE FROM run WHERE id IN (\
       SELECT id FROM run WHERE dir_id = ?1 \
       ORDER BY runs ASC, last_at ASC \
       LIMIT MAX(0, (SELECT COUNT(*) FROM run WHERE dir_id = ?1) - ?2))";

impl Track {
    /// Truncate the write-ahead log to nothing.
    ///
    /// Safe to call at any time and cheap when there is nothing to write back. Answers whether the
    /// pragma was accepted, which is what the tests assert on; no caller acts on a `false`, because
    /// a log that could not be checkpointed is a larger file and not a broken shell.
    pub fn checkpoint(&self) -> bool {
        runtime().block_on(async {
            let Ok(mut rows) = self.conn.query("PRAGMA wal_checkpoint(TRUNCATE)", ()).await else {
                return false;
            };
            // The row has to be *taken*, not merely offered. `query` in turso 0.7.2 is lazy: it
            // prepares the statement and hands back a cursor, and a pragma that is never stepped
            // never runs. Asking for the row and dropping it is what makes this a checkpoint rather
            // than an expensive no-op — measured, because the version that ignored the return value
            // left the log at 1.8 MB and looked like it had worked.
            rows.next().await.is_ok()
        })
    }

    /// Run the sweep now, whatever the stamp says, and checkpoint after it.
    ///
    /// Answers how many `run` rows it removed, which is only of interest to a test — nothing in the
    /// shell branches on it.
    pub fn sweep(&self) -> u64 {
        if !self.writable {
            return 0;
        }
        let removed = runtime().block_on(async {
            let at = now();
            let mut removed = self
                .conn
                .execute(OLD_RUNS, (at - RUN_MAX_AGE,))
                .await
                .unwrap_or(0);
            removed += cap_directories(&self.conn).await;
            forget_vanished(&self.conn, at).await;
            let _ = self.conn.execute(WRITE_STAMP, (at,)).await;
            removed
        });
        self.checkpoint();
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
        let last = runtime()
            .block_on(async { stamp(&self.conn).await })
            .unwrap_or(0);
        now().saturating_sub(last) >= SWEEP_EVERY
    }
}

/// Hand a due sweep to a thread and let the prompt come up.
///
/// The `&'static` is what makes this safe to detach: the store is the process-global installed by
/// the interactive loop and lives as long as the process does, so there is no join to wait on and
/// nothing to outlive. A shell that exits mid-sweep loses a sweep, which the next one redoes.
pub fn sweep_soon(track: &'static Track) {
    if !track.sweep_is_due() {
        return;
    }
    std::thread::spawn(move || {
        track.sweep();
    });
}

/// `meta.last_prune`, or `None` in a store that has never been swept.
async fn stamp(conn: &Connection) -> Option<i64> {
    let mut rows = conn.query(READ_STAMP, ()).await.ok()?;
    match rows.next().await {
        Ok(Some(row)) => match row.get_value(0) {
            Ok(Value::Integer(at)) => Some(at),
            _ => None,
        },
        _ => None,
    }
}

/// `stat()` every directory, note the ones that are gone, forget the ones that have been gone long
/// enough.
///
/// Two phases and two statements on purpose. Noting is idempotent — `missing_since IS NULL` means a
/// directory that came back and was re-visited has had its mark cleared by the visit statement, so
/// a disk that is late to mount costs the store nothing at all.
async fn forget_vanished(conn: &Connection, at: i64) {
    let Ok(mut rows) = conn.query("SELECT id, path FROM dir", ()).await else {
        return;
    };
    let mut gone = Vec::new();
    while let Ok(Some(row)) = rows.next().await {
        let (Ok(Value::Integer(id)), Ok(Value::Text(path))) = (row.get_value(0), row.get_value(1))
        else {
            continue;
        };
        if !std::path::Path::new(&path).is_dir() {
            gone.push(id);
        }
    }
    for id in gone {
        let _ = conn.execute(NOTE_MISSING, (at, id)).await;
    }
    let _ = conn.execute(DROP_MISSING, (at - GONE_MAX_AGE,)).await;
}

/// Bring every directory back under [`RUNS_PER_DIR`].
///
/// Only directories actually over the cap are touched, so this is one grouped read and then nothing
/// at all for the overwhelming majority of stores.
async fn cap_directories(conn: &Connection) -> u64 {
    let Ok(mut rows) = conn
        .query(
            "SELECT dir_id FROM run GROUP BY dir_id HAVING COUNT(*) > ?1",
            (RUNS_PER_DIR,),
        )
        .await
    else {
        return 0;
    };
    let mut over = Vec::new();
    while let Ok(Some(row)) = rows.next().await {
        if let Ok(Value::Integer(id)) = row.get_value(0) {
            over.push(id);
        }
    }
    let mut removed = 0;
    for id in over {
        removed += conn
            .execute(OVER_CAP, (id, RUNS_PER_DIR))
            .await
            .unwrap_or(0);
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::super::db::fixture::*;
    use super::*;
    use crate::track::{Run, Step, Visit};

    /// Backdate a row to `age` seconds ago, which is the only way to test a rule written in days.
    fn age_run(track: &Track, argv: &str, age: i64) {
        runtime().block_on(async {
            track
                .conn
                .execute(
                    "UPDATE run SET last_at = ?1 WHERE argv = ?2",
                    (now() - age, argv),
                )
                .await
                .expect("the row is backdated");
        });
    }

    /// The rule that bounds the table, and the two halves of it that must not be confused: run once
    /// and forgotten is rubbish; run twice, however long ago, is a habit.
    #[test]
    fn a_line_run_once_months_ago_goes_and_one_run_twice_stays() {
        let (_dir, track) = store();
        track.record(&ran("/w/alpha", "git commit -m wip", 0));
        track.record(&ran("/w/alpha", "cargo build", 0));
        track.record(&ran("/w/alpha", "cargo build", 0));
        age_run(&track, "git commit -m wip", RUN_MAX_AGE + 60);
        age_run(&track, "cargo build", RUN_MAX_AGE + 60);

        assert_eq!(track.sweep(), 1);
        assert_eq!(
            row(&track, "SELECT argv FROM run"),
            Some(Value::Text("cargo build".to_string())),
            "the one-off went; the habit stayed however old it is"
        );
    }

    /// A line run once but run *recently* is the line you are about to run again.
    #[test]
    fn a_recent_one_off_is_left_alone() {
        let (_dir, track) = store();
        track.record(&ran("/w/alpha", "kill 12345", 0));
        assert_eq!(track.sweep(), 0);
        assert_eq!(count(&track, "SELECT COUNT(*) FROM run"), 1);
    }

    /// An unmounted disk is not a deleted directory, so a directory that has gone is noted and kept.
    /// It is only forgotten once it has been gone for a month, and then its lines go with it.
    #[test]
    fn a_directory_that_vanished_is_noted_first_and_forgotten_later() {
        let (dir, track) = store();
        let here = dir.path().to_string_lossy().into_owned();
        track.record(&Step {
            ran_in: Visit::at(&here),
            moved_to: None,
            dwell_ms: 0,
            run: Some(Run {
                argv: "cargo test",
                mode: SH,
                status: Some(0),
                duration_ms: 1,
            }),
        });
        track.record(&ran("/w/gone-4e91", "cargo build", 0));

        track.sweep();
        assert_eq!(
            count(&track, "SELECT COUNT(*) FROM dir"),
            2,
            "noted as missing, not dropped — the disk may come back"
        );
        assert_eq!(
            count(
                &track,
                "SELECT COUNT(*) FROM dir WHERE missing_since IS NOT NULL"
            ),
            1,
            "and only the one that is actually gone is marked"
        );

        // A month later, with the directory still absent.
        runtime().block_on(async {
            track
                .conn
                .execute(
                    "UPDATE dir SET missing_since = ?1 WHERE path = '/w/gone-4e91'",
                    (now() - GONE_MAX_AGE - 60,),
                )
                .await
                .expect("the mark is backdated");
        });
        track.sweep();
        assert_eq!(count(&track, "SELECT COUNT(*) FROM dir"), 1);
        assert_eq!(
            count(
                &track,
                "SELECT COUNT(*) FROM run WHERE argv = 'cargo build'"
            ),
            0,
            "the cascade took the gone directory's lines with it, which needs foreign keys on"
        );
        assert_eq!(
            count(&track, "SELECT COUNT(*) FROM run WHERE argv = 'cargo test'"),
            1,
            "and took nothing belonging to the directory that is still there"
        );
    }

    /// One directory cannot grow without bound, whatever the calendar says. The rows that survive
    /// are the ones that were run most, not the ones that happen to have been inserted last.
    #[test]
    fn a_single_directory_cannot_grow_past_the_cap() {
        let (_dir, track) = store();
        for i in 0..RUNS_PER_DIR + 20 {
            let argv = format!("echo line-{i:04}");
            track.record(&Step {
                ran_in: Visit::at("/w/busy"),
                moved_to: None,
                dwell_ms: 0,
                run: Some(Run {
                    argv: &argv,
                    mode: SH,
                    status: Some(0),
                    duration_ms: 1,
                }),
            });
        }
        // One line the user actually leans on, and it is the newest, so an eviction that went by
        // age alone would take it.
        for _ in 0..5 {
            track.record(&ran("/w/busy", "make verify", 0));
        }

        track.sweep();
        assert_eq!(count(&track, "SELECT COUNT(*) FROM run"), RUNS_PER_DIR);
        assert_eq!(
            count(
                &track,
                "SELECT COUNT(*) FROM run WHERE argv = 'make verify'"
            ),
            1,
            "the habit survived; the once-run filler around it did not"
        );
    }

    /// The sweep is daily, and a store that has never been swept is due at once rather than a day
    /// after a version that stamps it first ran.
    #[test]
    fn the_sweep_is_due_once_a_day_and_immediately_on_a_store_that_has_never_had_one() {
        let (_dir, track) = store();
        assert!(track.sweep_is_due());
        track.sweep();
        assert!(!track.sweep_is_due(), "and not again for a day");

        runtime().block_on(async {
            track
                .conn
                .execute(WRITE_STAMP, (now() - SWEEP_EVERY - 60,))
                .await
                .expect("the stamp is backdated");
        });
        assert!(track.sweep_is_due());
    }

    /// The log that never truncates on its own, truncated. Without this it outgrows the database it
    /// belongs to and becomes the largest file the shell keeps.
    #[test]
    fn the_write_ahead_log_is_taken_back_to_nothing() {
        let (dir, track) = store();
        for i in 0..400 {
            let argv = format!("echo {i}");
            track.record(&Step {
                ran_in: Visit::at("/w/alpha"),
                moved_to: None,
                dwell_ms: 0,
                run: Some(Run {
                    argv: &argv,
                    mode: SH,
                    status: Some(0),
                    duration_ms: 1,
                }),
            });
        }
        let wal = dir.path().join("nested/track.db-wal");
        let before = std::fs::metadata(&wal).map(|m| m.len()).unwrap_or(0);
        assert!(before > 0, "there is a log to truncate in the first place");

        assert!(track.checkpoint());
        let after = std::fs::metadata(&wal).map(|m| m.len()).unwrap_or(0);
        assert_eq!(after, 0, "TRUNCATE means zero, not merely smaller");

        // And the data is still there afterwards, which is the half a checkpoint could get wrong.
        assert_eq!(count(&track, "SELECT COUNT(*) FROM run"), 400);
    }

    /// A file written by a version this binary does not understand is read, never rewritten — and
    /// deleting rows out of it would be the most destructive way to break that promise.
    #[test]
    fn a_store_from_a_newer_version_is_never_swept() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join("track.db");
        {
            let track = Track::open(&path).expect("the database opens");
            track.record(&ran("/w/alpha", "cargo build", 0));
            age_run(&track, "cargo build", RUN_MAX_AGE + 60);
            track.claim_future_version();
        }

        let newer = Track::open(&path).expect("it still opens");
        assert!(!newer.sweep_is_due());
        assert_eq!(newer.sweep(), 0);
        assert_eq!(count(&newer, "SELECT COUNT(*) FROM run"), 1);
    }
}

//! Three statements, one transaction, once per prompt.
//!
//! The REPL loop already computes every input this needs within sixty lines of each other and then
//! throws all but two of them away. So the whole write path is one call at the end of the loop
//! taking a struct built from locals that already exist; nothing is threaded through anything, and
//! nothing here runs on a worker thread. Measured at 81 µs against a 3000-directory database, which
//! is not something a shell can feel next to the fork and exec of the command itself — and a queue
//! would introduce a shutdown-drain problem for a store whose whole value is that it survives a
//! `kill -9` without corruption.
//!
//! # One open interval, and it is not in the database
//!
//! Time in a directory accumulates as `dwell_ms = dwell_ms + ?`, in SQL, from *closed* segments
//! only. A row with a null end is the thing that cannot survive a kill, so there isn't one: the
//! open segment is a clock mark held in the shell process, and the store is only ever told about
//! time that has already elapsed. Doing the addition in SQL rather than as a read-then-write in
//! Rust is also what lets two shells in one directory add up instead of clobbering each other.
//!
//! Note the unit honestly: this is shell-milliseconds, not wall-clock. Two shells sitting in one
//! directory for an hour record two hours. That is right for a ranking signal and wrong for a
//! report, and deduplicating it would need a session table and an interval-overlap computation on
//! read.

use super::db::{Run, Step, Track, Visit, capped, dir_id, now, runtime};
use super::redact;
use turso::Connection;

/// Count one arrival, or record the first.
const VISIT: &str = "INSERT INTO dir (path, base, root, visits, last_visit) \
     VALUES (?1, ?2, ?3, 1, ?4) \
     ON CONFLICT(path) DO UPDATE SET \
       visits = visits + 1, last_visit = excluded.last_visit, \
       root = excluded.root, missing_since = NULL";

/// Close the segment that just ended.
const DWELL: &str = "UPDATE dir SET dwell_ms = dwell_ms + ?1 WHERE id = ?2";

/// Fold one execution into the row for that line in that directory.
const RUN: &str = "INSERT INTO run \
       (dir_id, mode, argv, head, runs, fails, last_at, last_status, total_ms, max_ms) \
     VALUES (?1, ?2, ?3, ?4, 1, ?5, ?6, ?7, ?8, ?8) \
     ON CONFLICT(dir_id, mode, argv) DO UPDATE SET \
       runs = runs + 1, \
       fails = fails + excluded.fails, \
       last_at = excluded.last_at, \
       last_status = excluded.last_status, \
       total_ms = total_ms + excluded.total_ms, \
       max_ms = MAX(max_ms, excluded.max_ms)";

impl Track {
    /// Tell the store where the shell is standing, without counting it as a visit.
    ///
    /// Called once at startup with `$PWD`. Starting a shell somewhere is not the same act as
    /// walking there, so it must not raise that directory's rank — but the first command of the
    /// session still needs a `dir_id` to be attributed to, and the visit statement only runs when
    /// the directory *changes*.
    pub fn prime(&self, at: &Visit<'_>) -> bool {
        if !self.writable || redact::is_excluded(at.path, self.home.as_deref()) {
            self.forget_current();
            return false;
        }
        match runtime().block_on(async { dir_id(&self.conn, at).await }) {
            Some(id) => {
                self.remember_current(id, at.path);
                true
            }
            None => false,
        }
    }

    /// Write down one turn of the loop.
    ///
    /// The statements go in one transaction so that a crash between them cannot leave a run
    /// attributed to a directory whose arrival was never counted.
    pub fn record(&self, step: &Step<'_>) -> bool {
        if !self.writable {
            return false;
        }
        let here_excluded = redact::is_excluded(step.ran_in.path, self.home.as_deref());
        // Leaving an excluded directory for one worth remembering is still a real arrival, so the
        // two halves are gated separately rather than the whole step being dropped.
        let moved_to = step
            .moved_to
            .filter(|to| to.path != step.ran_in.path)
            .filter(|to| !redact::is_excluded(to.path, self.home.as_deref()));
        if here_excluded && moved_to.is_none() {
            return false;
        }

        let cached = self.cached_id(step.ran_in.path);
        // The lock is deliberately not held across the await: where the shell ended up comes back
        // out of the transaction and is stored afterwards.
        let outcome = runtime().block_on(async {
            let Ok(tx) = self.conn.unchecked_transaction().await else {
                return None;
            };
            match self.write(&tx, step, cached, here_excluded, moved_to).await {
                Some(next) => tx.commit().await.ok().map(|()| next),
                // Rolled back here rather than left to drop: a dangling transaction is undone on
                // the connection's *next* use, which would otherwise be somebody's read.
                None => {
                    let _ = tx.rollback().await;
                    None
                }
            }
        });

        let Some(next) = outcome else {
            return false;
        };
        match next {
            Some((id, path)) => self.remember_current(id, &path),
            None => self.forget_current(),
        }
        true
    }

    /// The statements themselves. Answers where the shell now is, or `None` if anything failed.
    async fn write(
        &self,
        conn: &Connection,
        step: &Step<'_>,
        cached: Option<i64>,
        here_excluded: bool,
        moved_to: Option<Visit<'_>>,
    ) -> Option<Option<(i64, String)>> {
        let at = now();
        let mut here = None;
        if !here_excluded {
            let id = match cached {
                Some(id) => id,
                None => dir_id(conn, &step.ran_in).await?,
            };
            here = Some(id);

            let dwell = capped(step.dwell_ms);
            if dwell > 0 {
                conn.execute(DWELL, (dwell, id)).await.ok()?;
            }
            if let Some(run) = step.run {
                record_run(conn, id, &run, at).await?;
            }
        }

        let Some(to) = moved_to else {
            return Some(here.map(|id| (id, step.ran_in.path.to_string())));
        };
        conn.execute(VISIT, (to.path, to.base(), to.root, at))
            .await
            .ok()?;
        Some(Some((dir_id(conn, &to).await?, to.path.to_string())))
    }

    /// The cached `dir.id`, but only if it is still the directory being asked about.
    fn cached_id(&self, path: &str) -> Option<i64> {
        self.current
            .lock()
            .ok()
            .and_then(|current| match current.as_ref() {
                Some((id, cached)) if cached == path => Some(*id),
                _ => None,
            })
    }

    fn remember_current(&self, id: i64, path: &str) {
        if let Ok(mut current) = self.current.lock() {
            *current = Some((id, path.to_string()));
        }
    }

    fn forget_current(&self) {
        if let Ok(mut current) = self.current.lock() {
            *current = None;
        }
    }
}

/// One execution, folded into its row.
async fn record_run(conn: &Connection, dir: i64, run: &Run<'_>, at: i64) -> Option<()> {
    let (argv, head) = redact::prepare(run.argv);
    if argv.is_empty() {
        // Not a failure: a line with no command in it has nothing to remember.
        return Some(());
    }
    let failed = i64::from(run.status.is_some_and(|status| status != 0));
    conn.execute(
        RUN,
        (
            dir,
            run.mode,
            argv,
            head,
            failed,
            at,
            run.status.map(i64::from),
            capped(run.duration_ms),
        ),
    )
    .await
    .ok()?;
    Some(())
}

#[cfg(test)]
mod tests {
    use super::super::db::CAP_MS;
    use super::super::db::fixture::*;
    use super::*;

    /// Somewhere the shell has been twice is one row that says two, not two rows.
    #[test]
    fn a_second_visit_counts_rather_than_duplicates() {
        let (_dir, track) = store();
        let alpha = Visit::at("/w/alpha");
        let beta = Visit::at("/w/beta");

        assert!(track.prime(&alpha));
        assert_eq!(
            count(&track, "SELECT visits FROM dir WHERE path = '/w/alpha'"),
            0,
            "starting a shell somewhere is not walking there"
        );

        for _ in 0..2 {
            assert!(track.record(&Step {
                ran_in: alpha,
                moved_to: Some(beta),
                dwell_ms: 0,
                run: None,
            }));
            assert!(track.record(&Step {
                ran_in: beta,
                moved_to: Some(alpha),
                dwell_ms: 0,
                run: None,
            }));
        }

        assert_eq!(
            count(&track, "SELECT COUNT(*) FROM dir"),
            2,
            "two directories, four arrivals"
        );
        assert_eq!(
            count(&track, "SELECT visits FROM dir WHERE path = '/w/beta'"),
            2
        );
        assert_eq!(
            count(&track, "SELECT visits FROM dir WHERE path = '/w/alpha'"),
            2
        );
        assert!(count(&track, "SELECT last_visit FROM dir WHERE path = '/w/beta'") > 0);
    }

    /// Going nowhere is not an arrival, however many commands are run standing still.
    #[test]
    fn staying_put_is_not_a_visit() {
        let (_dir, track) = store();
        for _ in 0..3 {
            track.record(&Step {
                ran_in: Visit::at("/w/alpha"),
                moved_to: Some(Visit::at("/w/alpha")),
                dwell_ms: 0,
                run: None,
            });
        }
        assert_eq!(
            count(&track, "SELECT visits FROM dir WHERE path = '/w/alpha'"),
            0
        );
    }

    /// Time is added in SQL from closed segments, and one contribution can never be a suspended
    /// laptop's worth.
    #[test]
    fn dwell_accumulates_and_is_capped() {
        let (_dir, track) = store();
        let here = Visit::at("/w/alpha");
        for dwell in [1_000, 2_500, 9 * 60 * 60 * 1000] {
            assert!(track.record(&Step {
                ran_in: here,
                moved_to: None,
                dwell_ms: dwell,
                run: None,
            }));
        }
        assert_eq!(
            count(&track, "SELECT dwell_ms FROM dir WHERE path = '/w/alpha'"),
            1_000 + 2_500 + CAP_MS,
            "nine hours of suspend counts as fifteen minutes"
        );
    }

    /// The aggregate: repeats are the entire point and cost nothing after the first.
    #[test]
    fn a_repeated_command_aggregates_into_one_row() {
        let (_dir, track) = store();
        track.record(&ran("/w/alpha", "cargo build", 0));
        track.record(&ran("/w/alpha", "cargo build", 1));
        track.record(&ran("/w/alpha", "cargo build", 0));

        assert_eq!(count(&track, "SELECT COUNT(*) FROM run"), 1);
        assert_eq!(count(&track, "SELECT runs FROM run"), 3);
        assert_eq!(count(&track, "SELECT fails FROM run"), 1);
        assert_eq!(
            count(&track, "SELECT last_status FROM run"),
            0,
            "the newest status wins"
        );
        assert_eq!(count(&track, "SELECT total_ms FROM run"), 15);
        assert_eq!(count(&track, "SELECT max_ms FROM run"), 5);
        assert_eq!(
            row(&track, "SELECT head FROM run"),
            Some(turso::Value::Text("cargo build".to_string())),
            "the tool and what it was doing, not the whole line"
        );
    }

    /// A command whose exit was never seen is not a command that succeeded.
    #[test]
    fn an_unobserved_exit_is_stored_as_unknown() {
        let (_dir, track) = store();
        track.record(&Step {
            ran_in: Visit::at("/w/alpha"),
            moved_to: None,
            dwell_ms: 0,
            run: Some(Run {
                argv: "sleep 100",
                mode: SH,
                status: None,
                duration_ms: 1,
            }),
        });
        assert_eq!(
            row(&track, "SELECT last_status FROM run"),
            Some(turso::Value::Null)
        );
        assert_eq!(count(&track, "SELECT fails FROM run"), 0);
    }

    /// A secret in the arguments costs the arguments. The directory, the count and the timing all
    /// survive, which is what makes an imperfect filter acceptable.
    #[test]
    fn a_redacted_line_still_leaves_its_timing_behind() {
        let (_dir, track) = store();
        track.record(&ran("/w/alpha", "curl --token abcdef https://x", 0));

        assert_eq!(
            row(&track, "SELECT argv FROM run"),
            Some(turso::Value::Text("curl".to_string()))
        );
        assert_eq!(count(&track, "SELECT total_ms FROM run"), 5);
        assert_eq!(
            count(&track, "SELECT COUNT(*) FROM dir WHERE path = '/w/alpha'"),
            1
        );
    }

    /// The privacy rule that is about places rather than about words.
    #[test]
    fn an_excluded_directory_leaves_no_trace() {
        let (_dir, track) = store();
        assert!(!track.record(&ran("/w/p/node_modules/react", "npm test", 0)));
        assert_eq!(count(&track, "SELECT COUNT(*) FROM dir"), 0);
        assert_eq!(count(&track, "SELECT COUNT(*) FROM run"), 0);

        // Leaving one is still a real arrival somewhere worth remembering.
        assert!(track.record(&Step {
            ran_in: Visit::at("/w/p/node_modules/react"),
            moved_to: Some(Visit::at("/w/p")),
            dwell_ms: 5_000,
            run: Some(Run {
                argv: "cd ../..",
                mode: SH,
                status: Some(0),
                duration_ms: 1,
            }),
        }));
        assert_eq!(
            count(&track, "SELECT COUNT(*) FROM run"),
            0,
            "not the command, though, and not its time"
        );
        assert_eq!(
            count(&track, "SELECT visits FROM dir WHERE path = '/w/p'"),
            1
        );
    }

    /// The bug the naive version has: the run needs a `dir_id` for a directory whose arrival was
    /// never recorded, because the shell started there.
    #[test]
    fn the_first_command_of_a_session_has_somewhere_to_be_attributed_to() {
        let (_dir, track) = store();
        assert!(track.record(&ran("/w/alpha", "cargo build", 0)));
        assert_eq!(count(&track, "SELECT COUNT(*) FROM run"), 1);
        assert_eq!(
            count(&track, "SELECT visits FROM dir WHERE path = '/w/alpha'"),
            0,
            "resolved, not visited"
        );
    }
}

//! Opening the store, and the shapes everything else is written in.
//!
//! # Why the async is hidden here
//!
//! `turso` is the pure-Rust rewrite of SQLite and its API is async; oslo's REPL is not. Rather than
//! colour the shell async, every call blocks on a small current-thread runtime owned by this
//! module, exactly as `startup::history_db` does. The runtime is built once: one per call would be
//! a thread and an epoll set per command.
//!
//! # Every failure answers nothing
//!
//! Copied verbatim from `history_db`: a shell whose tracker will not open is a working shell with a
//! dumber `cd`. Nothing here returns an error, because there is no caller that could act on one —
//! and every byte of this file is derived from use and rebuildable by more of it, which is what
//! makes `rm ~/.local/share/oslo/track.db` a supported repair rather than data loss.
//!
//! # Versioning
//!
//! `PRAGMA user_version`, mirrored into `meta.schema` so the file is legible by hand — the same
//! argument `history_db` makes for storing the mode as text. This is the one place the store
//! deliberately departs from `history_db`'s "there is no migration step", which is right for three
//! columns that will never change and wrong for a store other tools are meant to read. Migration is
//! additive only: `ALTER TABLE ... ADD COLUMN`, never destructive, and a file written by a version
//! this binary does not understand is read but never written.
//!
//! # The file is private, and so are its sidecars
//!
//! Every command line and every directory in here is plaintext, so the file is 0600 — and the
//! `-wal` beside it is 0600 too, which is the half that is easy to get wrong. `super::private`
//! holds that, and the note there gives the ordering it depends on.

use super::private::{make_private, repair, sidecars};
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use turso::{Connection, Value};

/// The schema this binary writes.
pub const SCHEMA_VERSION: i64 = 1;

/// How long a writer waits for a lock another shell is holding.
///
/// Small on purpose. Contention blocks a writer for exactly this long and then fails; a large
/// timeout turns contention into a visible stall at the prompt, a small one turns it into one
/// dropped sample. For a statistics table that is by far the cheaper failure.
const BUSY_MS: u64 = 250;

/// The largest single contribution to `dwell_ms` or `total_ms`.
///
/// A laptop suspend must not record nine hours in `~`, and `Instant::elapsed` on a command counts
/// the time it spent stopped under Ctrl-Z, which would poison a mean just as thoroughly.
pub(super) const CAP_MS: i64 = 15 * 60 * 1000;

/// The tables and their indexes.
///
/// `run` is keyed on `(dir_id, mode, argv)` — the whole line, not the command word, because `cargo
/// run --example xyz` here and `--example abc` there are the entire feature, and because a Lua line
/// and a shell line are not alternatives for the same slot. `head` is denormalised alongside so
/// that "what does cargo cost me, everywhere" is an index-backed read rather than a string split
/// over every row. `dir.base` is the lowercased final component, folded once at write time, which
/// is what makes matching a directory by name O(log n) instead of zoxide's full scan.
const SCHEMA: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS meta (\
       key TEXT PRIMARY KEY, \
       value INTEGER NOT NULL)",
    "CREATE TABLE IF NOT EXISTS dir (\
       id INTEGER PRIMARY KEY AUTOINCREMENT, \
       path TEXT NOT NULL UNIQUE, \
       base TEXT NOT NULL, \
       root TEXT, \
       visits INTEGER NOT NULL DEFAULT 0, \
       last_visit INTEGER NOT NULL DEFAULT 0, \
       dwell_ms INTEGER NOT NULL DEFAULT 0, \
       missing_since INTEGER)",
    "CREATE INDEX IF NOT EXISTS dir_base ON dir(base)",
    "CREATE INDEX IF NOT EXISTS dir_root ON dir(root)",
    "CREATE TABLE IF NOT EXISTS run (\
       id INTEGER PRIMARY KEY AUTOINCREMENT, \
       dir_id INTEGER NOT NULL REFERENCES dir(id) ON DELETE CASCADE, \
       mode TEXT NOT NULL, \
       argv TEXT NOT NULL, \
       head TEXT NOT NULL, \
       runs INTEGER NOT NULL DEFAULT 0, \
       fails INTEGER NOT NULL DEFAULT 0, \
       last_at INTEGER NOT NULL DEFAULT 0, \
       last_status INTEGER, \
       total_ms INTEGER NOT NULL DEFAULT 0, \
       max_ms INTEGER NOT NULL DEFAULT 0)",
    "CREATE UNIQUE INDEX IF NOT EXISTS run_key ON run(dir_id, mode, argv)",
    "CREATE INDEX IF NOT EXISTS run_argv ON run(mode, argv)",
    "CREATE INDEX IF NOT EXISTS run_head ON run(head, last_at)",
    // Partial, because the only rows the prune sweep is interested in are the ones run exactly
    // once. It stays correct as the upsert takes a row from one run to two: the row leaves the
    // index rather than lingering in it.
    "CREATE INDEX IF NOT EXISTS run_age ON run(last_at) WHERE runs = 1",
];

/// The runtime every call blocks on. See the module note.
pub(super) fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a current-thread runtime")
    })
}

/// Seconds since the epoch, as `dir.last_visit` and `run.last_at` store them.
pub(super) fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Milliseconds, floored at nothing and capped at [`CAP_MS`].
///
/// A backwards clock step contributes zero rather than a negative, which would otherwise take time
/// *away* from a directory.
pub(super) fn capped(ms: i64) -> i64 {
    ms.clamp(0, CAP_MS)
}

/// The exclusive upper end of the range of strings beginning with `prefix`.
///
/// `argv >= 'cargo run --ex' AND argv < 'cargo run --ey'` is a B-tree range scan; the `LIKE` that
/// expresses the same thing is not. SQLite compares text by bytes and UTF-8 byte order agrees with
/// code point order, so stepping the last character forward is exactly right.
///
/// `None` when there is nothing above the prefix — an empty one, or one ending at the top of the
/// code space. Callers read that as "no answer" and fall through to a wider search rather than
/// silently scanning.
pub(super) fn upper_bound(prefix: &str) -> Option<String> {
    let mut head = prefix.to_string();
    let last = head.pop()?;
    let mut code = u32::from(last) + 1;
    // Surrogates are not characters; stepping over the hole still names the next string.
    while char::from_u32(code).is_none() {
        code = code.checked_add(1)?;
        if code > u32::from(char::MAX) {
            return None;
        }
    }
    head.push(char::from_u32(code)?);
    Some(head)
}

/// A directory the shell stood in, and the worktree it belongs to.
///
/// `root` is the git toplevel, or `None` outside a repository. It is supplied by the caller rather
/// than computed here because the walk is one per directory *change*, not one per command.
#[derive(Debug, Clone, Copy)]
pub struct Visit<'a> {
    pub path: &'a str,
    pub root: Option<&'a str>,
}

impl<'a> Visit<'a> {
    /// A visit to `path`, outside any repository.
    pub fn at(path: &'a str) -> Self {
        Visit { path, root: None }
    }

    /// The match key: the final component, folded to lower case once, at write time.
    pub(super) fn base(&self) -> String {
        let trimmed = self.path.trim_end_matches('/');
        match trimmed.rsplit_once('/') {
            Some((_, base)) if !base.is_empty() => base.to_lowercase(),
            _ => trimmed.to_lowercase(),
        }
    }
}

/// One command, as the loop saw it finish.
///
/// `argv` must be a line that *parsed*: a typo is often a password typed into the wrong prompt, and
/// this store is not the place to find that out.
#[derive(Debug, Clone, Copy)]
pub struct Run<'a> {
    pub argv: &'a str,
    /// `history_db::MODE_SHELL` or `MODE_LUA`.
    pub mode: &'a str,
    /// `None` when the command's exit was never observed.
    pub status: Option<i32>,
    pub duration_ms: i64,
}

/// One turn of the REPL loop, from the store's point of view.
///
/// Everything in it is a local that `repl.rs` already has by the time the prompt comes back.
#[derive(Debug, Clone, Copy)]
pub struct Step<'a> {
    /// The directory the command ran in. The run is attributed here, not to where it left you.
    pub ran_in: Visit<'a>,
    /// Where the command left the shell, when that is somewhere else.
    pub moved_to: Option<Visit<'a>>,
    /// Milliseconds spent in `ran_in` since the last command boundary. Flushed every command
    /// rather than only at `cd`, so a `SIGKILL` costs seconds rather than hours.
    pub dwell_ms: i64,
    /// The command itself. `None` records the movement and the time without the line, which is
    /// what a boundary with nothing worth writing down looks like.
    pub run: Option<Run<'a>>,
}

/// An open store.
pub struct Track {
    pub(super) conn: Connection,
    /// False for a file written by a version this binary does not understand: keep reading it,
    /// stop writing to it. Dropping and recreating somebody else's data is never the answer.
    pub(super) writable: bool,
    /// The directory the shell is in and its `dir.id`.
    ///
    /// Without it the run insert has no `dir_id` on the first command of a session, because the
    /// visit statement only runs when the directory *changed*. A subselect there would yield null
    /// against a `NOT NULL` column; the write path resolves-or-inserts instead.
    pub(super) current: Mutex<Option<(i64, String)>>,
    /// `$HOME`, which is never a jump target and so is never recorded.
    pub(super) home: Option<String>,
}

impl Track {
    /// Open, creating the file and its directory if they are not there.
    ///
    /// The file and its `-wal` are made private *first*, before turso is handed the path and
    /// therefore before any statement runs. That ordering is the point rather than a detail. The
    /// schema statements below are the first real statements, and they are what brings the `-wal`
    /// into existence; tightening at the end of this function would leave it born world-readable,
    /// and an unclean shutdown then leaves the most recent commands sitting in a file anyone can
    /// read. Measured on turso 0.7.2: the sidecar does not inherit the database's mode at all — a
    /// 0600 database still produces a 0664 `-wal` under a 002 umask — so it is created here, at
    /// zero length, which is what an empty log is anyway.
    pub fn open(path: &Path) -> Option<Track> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok()?;
        }
        make_private(path)?;
        let [wal, shm] = sidecars(path);
        let _ = make_private(&wal);
        // Not created, only repaired: turso writes no `-shm`, and inventing one for it to find
        // would be this store guessing at another implementation's file format.
        repair(&shm);

        let file = path.to_str()?.to_string();
        let home = std::env::var("HOME").ok().filter(|home| !home.is_empty());
        let track = runtime().block_on(async move {
            let db = turso::Builder::new_local(&file).build().await.ok()?;
            let conn = db.connect().ok()?;
            conn.busy_timeout(Duration::from_millis(BUSY_MS)).ok()?;
            // Both of these are per *connection*, not per database, which is why the store holds
            // one open rather than taking a fresh one per call. They also have to go through a
            // query rather than an execute: a pragma answers with a row, and `execute` refuses one.
            conn.pragma_update("foreign_keys", "ON").await.ok()?;

            let found = user_version(&conn).await?;
            let writable = found <= SCHEMA_VERSION;
            if writable {
                for statement in SCHEMA {
                    conn.execute(*statement, ()).await.ok()?;
                }
                conn.pragma_update("user_version", SCHEMA_VERSION)
                    .await
                    .ok()?;
                conn.execute(
                    "INSERT INTO meta (key, value) VALUES ('schema', ?1) \
                     ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                    (SCHEMA_VERSION,),
                )
                .await
                .ok()?;
            }
            Some(Track {
                conn,
                writable,
                current: Mutex::new(None),
                home,
            })
        })?;

        // Best effort, and for a database an earlier version of this store created: anything that
        // appeared while the schema was being applied is tightened too. Nothing above depends on
        // this — the `-wal` was already private before turso saw it — which is why a failure here
        // is not allowed to refuse the store.
        for sidecar in sidecars(path) {
            repair(&sidecar);
        }
        Some(track)
    }

    /// Whether this store will accept writes. False only for a file from a future version.
    pub fn is_writable(&self) -> bool {
        self.writable
    }

    /// Stamp a schema version this binary does not understand, so that reopening the file produces
    /// the read-only store a newer oslo would have left behind.
    #[cfg(test)]
    pub(super) fn claim_future_version(&self) {
        runtime().block_on(async {
            self.conn
                .pragma_update("user_version", SCHEMA_VERSION + 1)
                .await
                .expect("the version is stamped");
        });
    }
}

/// The `dir.id` for a path, inserting an unvisited row if there is none.
///
/// The insert counts nothing: a directory reached this way was resolved because a command needed
/// somewhere to be attributed to, which is not the same act as walking there.
pub(super) async fn dir_id(conn: &Connection, at: &Visit<'_>) -> Option<i64> {
    if let Some(id) = lookup_dir(conn, at.path).await {
        return Some(id);
    }
    conn.execute(
        "INSERT INTO dir (path, base, root, visits, last_visit) VALUES (?1, ?2, ?3, 0, 0)",
        (at.path, at.base(), at.root),
    )
    .await
    .ok()?;
    lookup_dir(conn, at.path).await
}

async fn lookup_dir(conn: &Connection, path: &str) -> Option<i64> {
    let mut rows = conn
        .query("SELECT id FROM dir WHERE path = ?1", (path,))
        .await
        .ok()?;
    match rows.next().await {
        Ok(Some(row)) => match row.get_value(0) {
            Ok(Value::Integer(id)) => Some(id),
            _ => None,
        },
        _ => None,
    }
}

/// `PRAGMA user_version`, which is 0 for a file this store has never touched.
async fn user_version(conn: &Connection) -> Option<i64> {
    let mut rows = conn.query("PRAGMA user_version", ()).await.ok()?;
    match rows.next().await {
        Ok(Some(row)) => match row.get_value(0) {
            Ok(Value::Integer(version)) => Some(version),
            _ => Some(0),
        },
        _ => Some(0),
    }
}

/// A real database in a temporary directory, and the plainest way to look inside it.
///
/// Shared by the write and query tests, which both need to assert against columns rather than
/// against whatever the read API happens to expose.
#[cfg(test)]
pub(super) mod fixture {
    use super::*;

    pub const SH: &str = "sh";

    pub fn store() -> (tempfile::TempDir, Track) {
        let dir = tempfile::tempdir().expect("a temp dir");
        let track = Track::open(&dir.path().join("nested/track.db")).expect("the database opens");
        (dir, track)
    }

    /// A step that ran `argv` in `at` and went nowhere.
    pub fn ran(at: &'static str, argv: &'static str, status: i32) -> Step<'static> {
        Step {
            ran_in: Visit::at(at),
            moved_to: None,
            dwell_ms: 0,
            run: Some(Run {
                argv,
                mode: SH,
                status: Some(status),
                duration_ms: 5,
            }),
        }
    }

    pub fn row(track: &Track, sql: &str) -> Option<Value> {
        runtime().block_on(async {
            let mut rows = track.conn.query(sql, ()).await.ok()?;
            rows.next().await.ok()??.get_value(0).ok()
        })
    }

    pub fn count(track: &Track, sql: &str) -> i64 {
        match row(track, sql) {
            Some(Value::Integer(n)) => n,
            other => panic!("expected a count, got {other:?}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fixture::*;
    use super::*;

    #[test]
    fn opening_creates_what_is_missing_and_stamps_the_version() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join("nested/track.db");
        let track = Track::open(&path).expect("the database opens");

        assert!(
            path.exists(),
            "the directory is created too — a fresh machine has neither"
        );
        assert!(track.is_writable());
        assert_eq!(
            count(&track, "SELECT value FROM meta WHERE key = 'schema'"),
            SCHEMA_VERSION,
            "mirrored where a human reading the file by hand will find it"
        );
        assert_eq!(count(&track, "SELECT COUNT(*) FROM dir"), 0);
    }

    /// A store must never be the reason a shell will not start, and it must never be the reason
    /// somebody else's data is lost either.
    #[test]
    fn a_file_from_a_newer_version_is_read_but_not_written() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join("track.db");
        {
            let track = Track::open(&path).expect("the database opens");
            track.record(&ran("/w/alpha", "cargo build", 0));
            track.claim_future_version();
        }

        let newer = Track::open(&path).expect("it still opens");
        assert!(!newer.is_writable());
        assert!(!newer.record(&ran("/w/alpha", "cargo test", 0)));
        assert!(!newer.prime(&Visit::at("/w/alpha")));
        assert_eq!(
            newer.suggestion_here("/w/alpha", SH, "cargo"),
            Some("cargo build".to_string()),
            "reading is fine; overwriting is not"
        );
    }

    #[test]
    fn a_prefix_with_nothing_above_it_answers_nothing() {
        assert_eq!(
            upper_bound("cargo run --ex").as_deref(),
            Some("cargo run --ey")
        );
        assert_eq!(upper_bound("ab"), Some("ac".to_string()));
        assert_eq!(upper_bound(""), None);
        assert_eq!(upper_bound("\u{10FFFF}"), None);
        // Over the surrogate hole, which is not a character and cannot be pushed.
        assert_eq!(upper_bound("\u{D7FF}"), Some("\u{E000}".to_string()));
    }

    #[test]
    fn the_match_key_is_the_final_component_folded_once() {
        assert_eq!(Visit::at("/w/Rust").base(), "rust");
        assert_eq!(
            Visit::at("/w/Rust/").base(),
            "rust",
            "a trailing slash names the same place"
        );
        assert_eq!(Visit::at("/").base(), "");
        assert_eq!(Visit::at("relative").base(), "relative");
    }

    #[test]
    fn a_contribution_is_never_negative_and_never_a_suspended_laptop() {
        assert_eq!(capped(-1), 0, "a clock that went backwards adds nothing");
        assert_eq!(capped(1_000), 1_000);
        assert_eq!(capped(9 * 60 * 60 * 1000), CAP_MS);
    }
}

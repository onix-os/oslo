//! Opening the store, and the shapes everything else is written in.
//!
//! # There is no runtime here any more
//!
//! This module used to own a `tokio` current-thread runtime and block every call on it, because
//! `turso`'s API is async and oslo's REPL is not. The key-value store behind [`super::kv`] is
//! synchronous, so the runtime, both `OnceLock<Runtime>`s and every `block_on` went with turso.
//! What is left is ordinary function calls on the calling thread, which is what the shell wanted
//! all along.
//!
//! # Every failure answers nothing
//!
//! Unchanged, and inherited by the seam: a shell whose tracker will not open is a working shell
//! with a dumber `cd`. Nothing here returns an error, because there is no caller that could act on
//! one — and every byte of this file is derived from use and rebuildable by more of it, which is
//! what makes deleting it a supported repair rather than data loss.
//!
//! # Versioning
//!
//! `meta.schema`, a row like any other. `PRAGMA user_version` is gone with the SQL and it is not
//! missed: it was mirrored into `meta` anyway so that the file could be read by hand, and there is
//! now one copy instead of two that could disagree.
//!
//! The rule is the one the design gives and it is unchanged: a file written by a version this
//! binary does not understand is **read but never written**. That gate lives here, on
//! [`Track::is_writable`], and every write path asks it before it asks the store — deliberately at
//! this level rather than inside the seam, which is engine plumbing and has no opinion about what
//! oslo's rows mean.
//!
//! The version is stamped only when it is not already right, so a shell opening a store it has
//! opened a thousand times before pays a read and no `fsync` at all.
//!
//! # Directory ids are handed out here
//!
//! SQL had `INTEGER PRIMARY KEY AUTOINCREMENT`. A key-value store has no such thing, so
//! `next_id` keeps a counter in `meta` and increments it inside the same transaction that uses
//! it — which is what makes it safe between terminals, since the seam's `flock` means only one
//! writer exists at a time and the counter is read and written under it.
//!
//! Ids are never reused. A directory that is forgotten takes its id with it rather than freeing it
//! for the next one, because a stale index row pointing at a recycled id would attach one
//! directory's history to another. Eight bytes at 2^64 is not a number anybody reaches.

use super::kv::{Reader, Store, Tree, Writer};
use super::row::{DirRow, key};
use std::path::Path;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// The schema this binary writes.
pub const SCHEMA_VERSION: i64 = 2;

/// The largest single contribution to `dwell_ms` or `total_ms`.
///
/// A laptop suspend must not record nine hours in `~`, and `Instant::elapsed` on a command counts
/// the time it spent stopped under Ctrl-Z, which would poison a mean just as thoroughly.
pub(super) const CAP_MS: i64 = 15 * 60 * 1000;

/// `meta.schema`: what wrote this file.
pub(super) const SCHEMA: &str = "schema";

/// `meta.last_prune`: when the sweep last ran.
pub(super) const LAST_PRUNE: &str = "last_prune";

/// `meta.next_dir`: the next directory id to hand out.
const NEXT_DIR: &str = "next_dir";

/// Seconds since the epoch, as `dir.last_visit` and `run.last_at` store them.
pub(super) fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs() as i64)
        .unwrap_or(0)
}

/// Milliseconds, floored at nothing and capped at [`CAP_MS`].
///
/// A backwards clock step contributes zero rather than a negative, which would otherwise take time
/// *away* from a directory.
pub(super) fn capped(ms: i64) -> i64 {
    ms.clamp(0, CAP_MS)
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
    /// A path and a promise about the file — no descriptor, no lock, no map. See the seam's note:
    /// holding a handle would take a blocking `flock` for the life of the shell and hang the next
    /// terminal at its prompt, for ever.
    pub(super) store: Store,
    /// False for a file written by a version this binary does not understand: keep reading it,
    /// stop writing to it. Dropping and recreating somebody else's data is never the answer.
    pub(super) writable: bool,
    /// Appends to the command log since it was last trimmed, so the trim is amortised rather than
    /// paid on every line. See [`Track::trim_soon`].
    pub(super) since_trim: std::sync::atomic::AtomicUsize,
    /// The directory the shell is in and its id.
    ///
    /// Without it the run insert has no directory on the first command of a session, because the
    /// visit only happens when the directory *changed*. The write path resolves-or-inserts instead
    /// of writing a row that points nowhere.
    pub(super) current: Mutex<Option<(u64, String)>>,
    /// `$HOME`, which is never a jump target and so is never offered as one.
    pub(super) home: Option<String>,
}

impl Track {
    /// Open, creating the file and its directory if they are not there.
    ///
    /// The privacy ordering — the directory closed first, the file tightened before anything can
    /// have been written through it — belongs to [`super::kv`] now, along with the header check
    /// that stops a SQLite file left by an older oslo from being handed to an engine that panics
    /// on one. Both are the seam's, because both are facts about the engine rather than about
    /// what oslo stores.
    pub fn open(path: &Path) -> Option<Track> {
        // A file here that is not ours — an older build's database, or something a disk corrupted
        // — is renamed aside rather than opened or deleted. Without that, `Store::open` refuses it
        // for ever and the shell silently has no history and no ranking until somebody finds the
        // file by hand. `rename` within one directory is atomic, so two terminals starting together
        // cannot both move it: the loser finds nothing at the source and does nothing.
        if path.is_file()
            && !crate::track::kv::is_a_database(path)
            && let Some(name) = path.file_name().and_then(|name| name.to_str())
        {
            let _ = std::fs::rename(path, path.with_file_name(format!("{name}.unreadable")));
        }
        let store = Store::open(path)?;
        let found = store.read(|r| Some(meta(r, SCHEMA).unwrap_or(0)))?;
        let writable = found <= SCHEMA_VERSION;
        if writable && found != SCHEMA_VERSION {
            store.write(|w| set_meta(w, SCHEMA, SCHEMA_VERSION))?;
        }
        Some(Track {
            store,
            writable,
            since_trim: std::sync::atomic::AtomicUsize::new(0),
            current: Mutex::new(None),
            home: std::env::var("HOME").ok().filter(|home| !home.is_empty()),
        })
    }

    /// Whether this store will accept writes. False only for a file from a future version.
    pub fn is_writable(&self) -> bool {
        self.writable
    }

    /// Stamp a schema version this binary does not understand, so that reopening the file produces
    /// the read-only store a newer oslo would have left behind.
    #[cfg(test)]
    pub(super) fn claim_future_version(&self) {
        self.store
            .write(|w| set_meta(w, SCHEMA, SCHEMA_VERSION + 1))
            .expect("the version is stamped");
    }
}

/// A `meta` value, or `None` for a name this store has never written.
pub(super) fn meta(reader: &Reader<'_, '_>, name: &str) -> Option<i64> {
    key::number_of(&reader.get(Tree::Meta, &key::meta(name))?)
}

pub(super) fn set_meta(writer: &Writer<'_, '_>, name: &str, value: i64) -> Option<()> {
    writer.put(Tree::Meta, key::meta(name), key::number(value))
}

/// The id for a path, or `None` if this store has never seen it.
pub(super) fn lookup_dir(reader: &Reader<'_, '_>, path: &str) -> Option<u64> {
    key::id_of(&reader.get(Tree::DirByPath, &key::by_path(path))?)
}

/// One directory's row.
pub(super) fn read_dir(reader: &Reader<'_, '_>, id: u64) -> Option<DirRow> {
    DirRow::decode(&reader.get(Tree::Dir, &key::dir(id))?)
}

/// The id for a directory, inserting an unvisited row if there is none.
///
/// The insert counts nothing: a directory reached this way was resolved because a command needed
/// somewhere to be attributed to, which is not the same act as walking there.
pub(super) fn resolve_dir(writer: &Writer<'_, '_>, at: &Visit<'_>) -> Option<u64> {
    if let Some(id) = lookup_dir(writer, at.path) {
        return Some(id);
    }
    insert_dir(writer, &DirRow::unvisited(at.path, at.base(), at.root))
}

/// Take the next id and write a directory under it, indexes and all.
pub(super) fn insert_dir(writer: &Writer<'_, '_>, row: &DirRow) -> Option<u64> {
    let id = next_id(writer)?;
    put_dir(writer, id, None, row)?;
    Some(id)
}

/// Write a directory's row and bring its three index buckets into line with it.
///
/// `was` is the row as it stood, or `None` for one being inserted. It is asked for rather than
/// re-read because every caller has just read it to modify it, and because the difference between
/// the two is exactly the index work that has to happen: there are no foreign keys here and no
/// triggers, so an index this function forgets is an index that silently rots.
///
/// `path` and `base` are not in that list. The path *is* the identity of a directory, so neither
/// can change without the row being a different row.
pub(super) fn put_dir(
    writer: &Writer<'_, '_>,
    id: u64,
    was: Option<&DirRow>,
    row: &DirRow,
) -> Option<()> {
    writer.put(Tree::Dir, key::dir(id), row.encode())?;
    if was.is_none() {
        writer.put(Tree::DirByPath, key::by_path(&row.path), key::id(id))?;
        writer.put(Tree::DirByBase, key::by_base(&row.base, id), Vec::new())?;
    }
    // A directory can change worktree — `git init` in it, a repository moved, a submodule turned
    // into a directory — and the visit statement refreshes `root` every time precisely so that it
    // does. The index has to follow, or a widened suggestion answers out of a repository the user
    // left months ago.
    let before = was.and_then(|was| was.root.as_deref());
    if before != row.root.as_deref() {
        if let Some(root) = before {
            writer.delete(Tree::DirByRoot, &key::by_root(root, id));
        }
        if let Some(root) = row.root.as_deref() {
            writer.put(Tree::DirByRoot, key::by_root(root, id), Vec::new())?;
        }
    }
    Some(())
}

/// The next directory id, taken from the counter and put back one higher.
///
/// Read and written inside the caller's transaction, so two terminals cannot be handed the same
/// id: the seam's `flock` means only one writer exists at a time, and a transaction that does not
/// commit does not consume a number either.
fn next_id(writer: &Writer<'_, '_>) -> Option<u64> {
    let next = meta(writer, NEXT_DIR).unwrap_or(1).max(1);
    set_meta(writer, NEXT_DIR, next + 1)?;
    Some(next as u64)
}

/// A real store in a temporary directory, and the plainest way to look inside it.
///
/// Shared by the write, query and prune tests, which all need to assert against rows rather than
/// against whatever the read API happens to expose. Under SQL these took a `SELECT`; a store with
/// no query language needs named accessors instead, which is a fair trade — `visits_of(&track,
/// "/w/alpha")` says what it wants, and a typo in it does not compile.
#[cfg(test)]
pub(super) mod fixture {
    use super::super::kv::Span;
    use super::super::row::{RunRow, span};
    use super::*;

    pub const SH: &str = "sh";

    pub fn store() -> (tempfile::TempDir, Track) {
        let dir = tempfile::tempdir().expect("a temp dir");
        let track = Track::open(&dir.path().join("nested/track.kv")).expect("the store opens");
        (dir, track)
    }

    /// The same store, told where home is.
    ///
    /// `Track::open` reads `$HOME` from the environment, and setting an environment variable in a
    /// test is a process-global write racing every other test on the libtest thread pool. This
    /// hands the value over directly instead.
    pub fn store_with_home(home: &str) -> (tempfile::TempDir, Track) {
        let (dir, mut track) = store();
        track.home = Some(home.to_string());
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

    pub fn dir_row(track: &Track, path: &str) -> Option<DirRow> {
        track.store.read(|r| read_dir(r, lookup_dir(r, path)?))
    }

    pub fn visits_of(track: &Track, path: &str) -> i64 {
        dir_row(track, path).map_or(-1, |row| row.visits)
    }

    pub fn run_row(track: &Track, path: &str, mode: &str, argv: &str) -> Option<RunRow> {
        track
            .store
            .read(|r| {
                let id = lookup_dir(r, path)?;
                Some(RunRow::decode(
                    &r.get(Tree::Run, &key::run(id, mode, argv))?,
                ))
            })
            .flatten()
    }

    /// Every line remembered in one directory, in key order.
    pub fn lines_in(track: &Track, path: &str) -> Vec<String> {
        track
            .store
            .read(|r| {
                let id = lookup_dir(r, path)?;
                Some(r.collect(Tree::Run, &span::runs_of(id), |key, _| {
                    super::super::row::field::argv_of_run(key).map(|argv| argv.into_owned())
                }))
            })
            .unwrap_or_default()
    }

    /// How many rows a bucket holds, which is what most of the old `SELECT COUNT(*)` asked.
    pub fn rows(track: &Track, tree: Tree) -> usize {
        track
            .store
            .read(|r| Some(r.count(tree, &Span::all())))
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::fixture::*;
    use super::*;

    #[test]
    fn opening_creates_what_is_missing_and_stamps_the_version() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join("nested/track.kv");
        let track = Track::open(&path).expect("the store opens");

        assert!(
            path.exists(),
            "the directory is created too — a fresh machine has neither"
        );
        assert!(track.is_writable());
        assert_eq!(
            track.store.read(|r| meta(r, SCHEMA)),
            Some(SCHEMA_VERSION),
            "written where a human reading the file by hand will find it"
        );
        assert_eq!(rows(&track, Tree::Dir), 0);
    }

    /// A store must never be the reason a shell will not start, and it must never be the reason
    /// somebody else's data is lost either.
    #[test]
    fn a_file_from_a_newer_version_is_read_but_not_written() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join("track.kv");
        {
            let track = Track::open(&path).expect("the store opens");
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

    /// The stamp is not rewritten on every open. It matters here in a way it did not under SQL: a
    /// write is an `fsync`, and paying one to say what the file already says would be a cost on
    /// every shell that ever starts.
    #[test]
    fn a_store_already_at_this_version_is_not_written_to_when_it_is_opened() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join("track.kv");
        let before = {
            let track = Track::open(&path).expect("the store opens");
            track.record(&ran("/w/alpha", "cargo build", 0));
            std::fs::metadata(&path).expect("it exists").modified().ok()
        };
        // Long enough that a write would land in a different tick, so the assertion below is about
        // nothing having been written rather than about the clock being coarse.
        std::thread::sleep(std::time::Duration::from_millis(20));

        let again = Track::open(&path).expect("it opens again");
        assert!(again.is_writable());
        assert_eq!(
            std::fs::metadata(&path).expect("it exists").modified().ok(),
            before,
            "the second open read the version and wrote nothing"
        );
    }

    /// Ids are handed out one at a time and never reused: a recycled id would attach one
    /// directory's remembered lines to whichever directory got the number next.
    #[test]
    fn every_directory_gets_its_own_id_and_no_id_is_ever_handed_out_twice() {
        let (_dir, track) = store();
        let seen: Vec<u64> = ["/w/alpha", "/w/beta", "/w/gamma"]
            .iter()
            .map(|path| {
                track
                    .store
                    .write(|w| resolve_dir(w, &Visit::at(path)))
                    .expect("resolved")
            })
            .collect();
        assert_eq!(seen, vec![1, 2, 3]);

        // Asking again is a lookup, not another id.
        assert_eq!(
            track
                .store
                .write(|w| resolve_dir(w, &Visit::at("/w/beta")))
                .expect("resolved"),
            2
        );
        assert_eq!(rows(&track, Tree::Dir), 3);
    }

    /// A directory that changes worktree must leave the old one's index behind, or a suggestion
    /// widened to a repository answers out of a repository the user left months ago.
    #[test]
    fn moving_a_directory_into_a_worktree_moves_its_index_row_with_it() {
        let (_dir, track) = store();
        fn inside(root: &'static str) -> Visit<'static> {
            Visit {
                path: "/w/alpha",
                root: Some(root),
            }
        }
        track.record(&Step {
            ran_in: Visit::at("/w"),
            moved_to: Some(inside("/w/old")),
            dwell_ms: 0,
            run: None,
        });
        track.record(&Step {
            ran_in: inside("/w/old"),
            moved_to: Some(Visit::at("/w")),
            dwell_ms: 0,
            run: None,
        });
        track.record(&Step {
            ran_in: Visit::at("/w"),
            moved_to: Some(inside("/w/new")),
            dwell_ms: 0,
            run: None,
        });

        assert_eq!(
            dir_row(&track, "/w/alpha").and_then(|row| row.root),
            Some("/w/new".to_string())
        );
        assert_eq!(
            rows(&track, Tree::DirByRoot),
            1,
            "one row, not one per worktree it has ever been in"
        );
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

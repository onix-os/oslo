//! The one-way step off turso, and the history it must not lose.
//!
//! `~/.local/share/oslo/history.db` is a *SQLite* file on every machine that has run a released
//! oslo. This build has no SQLite in it, so those bytes are unreadable here and there is no
//! migration to write — nothing in this process can parse a b-tree page of somebody else's format.
//!
//! What there is instead is a rule: **the file is moved aside, never opened and never deleted.**
//! Silently losing somebody's shell history is the worst outcome this work could have, and it is
//! also the quietest — a history that is simply empty on the first start of a new version looks
//! exactly like a history that was never there. So the old file is renamed to `history.db.turso`,
//! beside the new one, where a person who wants those lines back can still reach them with
//! `sqlite3` and where a rollback to a turso build would find them untouched.
//!
//! Two details that are not decoration:
//!
//! * **An existing `.turso` is never written over.** A second name is used instead
//!   (`history.db.turso.2` and so on), because a machine that has bounced between builds can have
//!   more than one of these and the first one is the oldest and the one worth most. When every name
//!   is taken this gives up and the shell runs without a history rather than overwrite anything.
//! * **The test is "not one of ours", not "is SQLite".** [`oslo::track::kv::is_a_database`] is the
//!   same header check `Store::open` uses, so anything this build would refuse gets preserved
//!   rather than left in the way — a truncated file, a half-copied one, a store from a future
//!   format. In practice it is turso's, which is what the suffix says.
//!
//! Doing nothing is always a safe answer here: the file stays, [`super::History::open`] answers
//! `None` on it, and the shell starts without a history rather than without the old one.

use std::path::{Path, PathBuf};

/// The suffix a history this build cannot read is kept under.
const ASIDE: &str = "turso";

/// How many of them may pile up before this gives up rather than overwrite one.
const KEEP_AT_MOST: u32 = 9;

/// Move whatever is at `path` aside if this build cannot read it, and answer where it went.
///
/// `None` means there was nothing to do — no file, or a file this build wrote — and also covers the
/// failures, all of which are "leave it exactly as it is".
pub(super) fn keep_a_history_this_build_cannot_read(path: &Path) -> Option<PathBuf> {
    // A directory, a socket or a device at this path is not a history and is not ours to rename.
    if !path.is_file() || oslo::track::kv::is_a_database(path) {
        return None;
    }
    let aside = a_free_name_beside(path)?;
    // `rename` within one directory is atomic, and the free-name check above is what keeps it from
    // replacing an earlier one. Two terminals starting at once cannot both move the same file: the
    // one that loses finds nothing at the source and fails, which is a no-op.
    std::fs::rename(path, &aside).ok()?;
    Some(aside)
}

/// The first unused `path.turso`, `path.turso.2`, … or `None` when they are all taken.
fn a_free_name_beside(path: &Path) -> Option<PathBuf> {
    for attempt in 1..=KEEP_AT_MOST {
        let mut name = path.as_os_str().to_os_string();
        name.push(match attempt {
            1 => format!(".{ASIDE}"),
            n => format!(".{ASIDE}.{n}"),
        });
        let candidate = PathBuf::from(name);
        if !candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use oslo::track::kv::Store;

    /// Enough of a SQLite file to be recognisably one and long enough that the header check reads
    /// it at all — which is what every existing user has at this path.
    fn a_history_turso_wrote(path: &Path) {
        let mut bytes = b"SQLite format 3\0".to_vec();
        bytes.resize(16 * 1024, 0);
        std::fs::write(path, &bytes).expect("written");
    }

    /// The point of the whole module: the lines are still on disk afterwards, under a name that
    /// says what wrote them.
    #[test]
    fn a_history_from_an_older_oslo_is_moved_aside_rather_than_destroyed() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join("history.db");
        a_history_turso_wrote(&path);

        let aside = keep_a_history_this_build_cannot_read(&path).expect("it is moved");
        assert_eq!(aside, dir.path().join("history.db.turso"));
        assert!(!path.exists(), "and the path is free for the new store");
        assert!(
            std::fs::read(&aside)
                .expect("still readable")
                .starts_with(b"SQLite format 3\0"),
            "the old bytes are exactly as they were"
        );
    }

    /// The common case, run on every start after the first: this must not touch a working history.
    #[test]
    fn a_history_this_build_wrote_is_left_where_it_is() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join("history.db");
        Store::open(&path).expect("a store of ours");

        assert_eq!(keep_a_history_this_build_cannot_read(&path), None);
        assert!(path.exists());
        assert!(!dir.path().join("history.db.turso").exists());
    }

    /// A machine that has bounced between builds has more than one of these, and the first is the
    /// oldest. Overwriting it would lose exactly what this module exists to keep.
    #[test]
    fn a_history_already_moved_aside_is_never_written_over() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join("history.db");
        let first = dir.path().join("history.db.turso");
        std::fs::write(&first, b"the first one, and the one worth most").expect("written");
        a_history_turso_wrote(&path);

        let aside = keep_a_history_this_build_cannot_read(&path).expect("it is moved");
        assert_eq!(aside, dir.path().join("history.db.turso.2"));
        assert_eq!(
            std::fs::read(&first).expect("still readable"),
            b"the first one, and the one worth most",
            "the earlier one is untouched"
        );
    }

    /// When there is nowhere left to put one, nothing is moved and nothing is lost — the shell
    /// runs without a history instead.
    #[test]
    fn a_history_is_kept_rather_than_replaced_once_every_name_is_taken() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join("history.db");
        a_history_turso_wrote(&path);
        for attempt in 1..=KEEP_AT_MOST {
            let name = match attempt {
                1 => "history.db.turso".to_string(),
                n => format!("history.db.turso.{n}"),
            };
            std::fs::write(dir.path().join(name), b"taken").expect("written");
        }

        assert_eq!(keep_a_history_this_build_cannot_read(&path), None);
        assert!(path.exists(), "and the unreadable history is still there");
    }

    /// A first run has nothing to move, and a path that is not a file is not this module's to
    /// rename.
    #[test]
    fn there_is_nothing_to_move_before_the_first_run() {
        let dir = tempfile::tempdir().expect("a temp dir");
        assert_eq!(
            keep_a_history_this_build_cannot_read(&dir.path().join("history.db")),
            None
        );
        assert_eq!(keep_a_history_this_build_cannot_read(dir.path()), None);
    }
}

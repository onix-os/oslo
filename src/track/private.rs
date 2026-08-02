//! Keeping the store to its owner.
//!
//! This database is a plaintext record of every command line the shell was told to remember and
//! every directory it has stood in. On a shared machine that is a reading of somebody's work, their
//! employer and their unreleased projects, and the default a fresh file gets — 0666 minus the
//! umask — is not an answer to it. So the file is 0600.
//!
//! # The sidecars, and the ordering they force
//!
//! The `-wal` beside the database is not scratch. After an unclean shutdown the most recent
//! commands are in it and nowhere else, so a world-readable `-wal` leaks exactly the newest and
//! most interesting rows. SQLite brings it into existence on the first real statement — which for
//! this store is the schema, inside [`Track::open`] — so a store that tightened its files on the
//! way *out* of `open` would already have written one at 0644 and would never notice.
//!
//! Measured on turso 0.7.2, and this is the part worth knowing: the sidecar does not take the
//! database's mode. A 0600 database still produces a 0664 `-wal` under a 002 umask, so tightening
//! the database first is *necessary and not sufficient*. Both files are therefore created here, at
//! zero length, before turso is handed the path — an empty `-wal` is an empty log, which is what a
//! database that does not exist yet has anyway.
//!
//! [`Track::open`]: super::db::Track::open

use std::fs::{OpenOptions, Permissions};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

/// The mode the store and its sidecars are kept at.
pub(super) const PRIVATE: u32 = 0o600;

/// What SQLite writes beside the database file.
const SIDECARS: [&str; 2] = ["-wal", "-shm"];

/// Make a file exist and be readable by nobody but its owner.
///
/// Two steps, because neither is enough alone. `create(true).mode(PRIVATE)` is what the file is
/// *born* with, and a umask can only take bits away from that, never add them — so the file never
/// exists, for an instant, at anything looser than 0600. The `set_permissions` afterwards is not
/// masked at all, so it also repairs a file an earlier version of this store left at 0644.
///
/// `None` when neither could be done. A store that cannot be made private is one this shell does
/// without: recording a year of somebody's commands somewhere anyone can read them is a worse
/// outcome than a dumber `cd`.
pub(super) fn make_private(path: &Path) -> Option<()> {
    OpenOptions::new()
        .create(true)
        .write(true)
        .mode(PRIVATE)
        .open(path)
        .ok()?;
    std::fs::set_permissions(path, Permissions::from_mode(PRIVATE)).ok()
}

/// Tighten a file that is already there, without bringing one into being that is not.
pub(super) fn repair(path: &Path) {
    if path.exists() {
        let _ = std::fs::set_permissions(path, Permissions::from_mode(PRIVATE));
    }
}

/// The `-wal` and `-shm` that belong to a database file, in that order.
pub(super) fn sidecars(path: &Path) -> [PathBuf; 2] {
    let name = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    SIDECARS.map(|suffix| path.with_file_name(format!("{name}{suffix}")))
}

#[cfg(test)]
mod tests {
    use super::super::db::Track;
    use super::super::db::fixture::{count, ran};
    use super::*;

    /// The permission bits of a file, with the type and setuid bits masked off.
    fn mode_of(path: &Path) -> u32 {
        std::fs::metadata(path)
            .unwrap_or_else(|e| panic!("{} should exist: {e}", path.display()))
            .permissions()
            .mode()
            & 0o777
    }

    /// The file is a plaintext record of what somebody runs and where they run it. On a shared
    /// machine the default 0666-minus-umask hands that to every other account on the box.
    #[test]
    fn the_store_and_its_sidecars_are_readable_by_nobody_else() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join("nested/track.db");
        let track = Track::open(&path).expect("the database opens");
        track.record(&ran("/w/alpha", "cargo build", 0));

        assert_eq!(mode_of(&path), PRIVATE, "the database itself");
        let mut found = 0;
        for sidecar in sidecars(&path) {
            if sidecar.exists() {
                found += 1;
                assert_eq!(
                    mode_of(&sidecar),
                    PRIVATE,
                    "{} holds the newest rows and nothing else does",
                    sidecar.display()
                );
            }
        }
        assert!(
            found > 0,
            "there is a sidecar, so there is one to get wrong"
        );
    }

    /// The ordering, pinned by the fact that forces it: the `-wal` is brought into existence by the
    /// first statement, and the first statement is the schema, inside `open`. So a store that
    /// tightened its files on the way *out* of `open` would already have written a world-readable
    /// one — and after an unclean shutdown that is where the most recent commands live.
    #[test]
    fn the_sidecar_is_already_there_when_open_returns() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join("track.db");
        let _track = Track::open(&path).expect("the database opens");

        let [wal, _shm] = sidecars(&path);
        assert!(wal.exists(), "written before open returned, not after");
        assert_eq!(mode_of(&wal), PRIVATE);
    }

    /// A store this version did not create is repaired rather than left as it was found — and
    /// repaired without being emptied, because the data in it is the user's.
    #[test]
    fn a_world_readable_database_from_an_earlier_version_is_repaired() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join("track.db");
        {
            let track = Track::open(&path).expect("the database opens");
            track.record(&ran("/w/alpha", "cargo build", 0));
        }
        let [wal, shm] = sidecars(&path);
        std::fs::write(&shm, b"").expect("a stray shm from some other tool");
        for loose in [&path, &wal, &shm] {
            std::fs::set_permissions(loose, Permissions::from_mode(0o644))
                .expect("loosened, as an older version left them");
        }

        let track = Track::open(&path).expect("it still opens");
        for tightened in [&path, &wal, &shm] {
            assert_eq!(
                mode_of(tightened),
                PRIVATE,
                "{} was left readable by an earlier version",
                tightened.display()
            );
        }
        assert_eq!(
            count(&track, "SELECT runs FROM run"),
            1,
            "repaired, not replaced"
        );
    }

    /// The ordering again, this time so that a regression cannot hide behind the repair sweep at
    /// the end of `open`. A file turso refuses — anything that is not a database — makes `open`
    /// answer `None` on the spot, and the sweep never runs. If the mode is right anyway then the
    /// file was made private *before* turso was handed it, which is the property being claimed;
    /// a store that tightened on the way out would leave this one exactly as it found it.
    #[test]
    fn the_file_is_private_before_anything_that_can_fail() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join("track.db");
        std::fs::write(&path, b"not a database, a text file somebody put here").expect("written");
        std::fs::set_permissions(&path, Permissions::from_mode(0o644)).expect("world-readable");

        assert!(Track::open(&path).is_none(), "turso will not have it");
        assert_eq!(
            mode_of(&path),
            PRIVATE,
            "tightened on the way in, where it counts, not on a way out that was never taken"
        );
    }

    /// The repair never conjures a file: a store with no `-shm` must not grow one, because turso
    /// writes none and inventing one is this store guessing at another implementation's format.
    #[test]
    fn repairing_what_is_not_there_creates_nothing() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let absent = dir.path().join("track.db-shm");
        repair(&absent);
        assert!(!absent.exists());
    }
}

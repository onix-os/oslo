//! Tracking database file validation and permissions.

use std::fs::{File, OpenOptions as FileOpenOptions, Permissions};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};

use tagdata::{DB, FormatInfo, OpenOptions};

/// The mode of the tracking database.
pub(super) const PRIVATE: u32 = 0o600;

/// The mode of the directory containing tracking data.
pub(super) const PRIVATE_DIR: u32 = 0o700;

/// The page size used for new tracking databases.
pub(super) const PAGE_SIZE: u64 = 4096;

pub(super) fn open(path: &Path) -> Option<DB> {
    if !is_a_database(path) {
        return None;
    }
    catch_unwind(AssertUnwindSafe(|| {
        OpenOptions::new().pagesize(PAGE_SIZE).open(path).ok()
    }))
    .ok()
    .flatten()
}

/// Locks database initialization.
pub(super) fn open_lock(path: &Path) -> Option<File> {
    prepare_directory(path)?;
    let lock_path = lock_path(path);
    let lock = FileOpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .mode(PRIVATE)
        .open(&lock_path)
        .ok()?;
    make_private(&lock_path)?;
    lock.lock().ok()?;
    Some(lock)
}

/// `hist.db` locks on `hist.lock`, `notes.kv` on `notes.lock`.
///
/// The extension is replaced rather than appended. It used to append, giving `hist.db.lock` beside
/// `hist.db` and `hist.model` — one of three files in a profile's directory wearing its own name
/// differently, for no reason beyond how the string was built.
fn lock_path(path: &Path) -> PathBuf {
    path.with_extension("lock")
}

/// Creates the database directory and restricts it to the current user.
pub(super) fn prepare_directory(path: &Path) -> Option<()> {
    let parent = path.parent()?;
    if !parent.as_os_str().is_empty() {
        std::fs::create_dir_all(parent).ok()?;
        std::fs::set_permissions(parent, Permissions::from_mode(PRIVATE_DIR)).ok()?;
    }
    Some(())
}

/// Restricts the database file to the current user.
pub(super) fn make_private(path: &Path) -> Option<()> {
    std::fs::set_permissions(path, Permissions::from_mode(PRIVATE)).ok()
}

pub(super) fn backup(db: &tagdata::DB, destination: &Path) -> Result<(), String> {
    if destination.exists() {
        return Err(format!("{}: output already exists", destination.display()));
    }
    let parent = destination
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut nonce = [0; 8];
    getrandom::fill(&mut nonce)
        .map_err(|error| format!("cannot name backup staging directory: {error}"))?;
    let staging_dir = parent.join(format!(
        ".oslo-backup-{}-{:016x}",
        std::process::id(),
        u64::from_ne_bytes(nonce)
    ));
    std::fs::DirBuilder::new()
        .mode(PRIVATE_DIR)
        .create(&staging_dir)
        .map_err(|error| format!("{}: {error}", staging_dir.display()))?;
    let staged = staging_dir.join("snapshot.kv");
    let result = (|| {
        db.backup_to(&staged)
            .map_err(|error| format!("{}: {error}", destination.display()))?;
        make_private(&staged)
            .ok_or_else(|| format!("{}: cannot set mode 0600", destination.display()))?;
        std::fs::hard_link(&staged, destination)
            .map_err(|error| format!("{}: {error}", destination.display()))?;
        if let Err(error) = std::fs::File::open(parent).and_then(|dir| dir.sync_all()) {
            let _ = std::fs::remove_file(destination);
            return Err(format!("{}: {error}", parent.display()));
        }
        Ok(())
    })();
    let _ = std::fs::remove_file(&staged);
    let _ = std::fs::remove_dir(&staging_dir);
    result
}

/// Whether `path` is absent or contains a supported Tagdata database.
pub fn is_a_database(path: &Path) -> bool {
    if !path.exists() {
        return true;
    }
    FormatInfo::inspect(path).is_ok_and(|info| info.page_size == PAGE_SIZE)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mode_of(path: &Path) -> u32 {
        std::fs::metadata(path)
            .unwrap_or_else(|error| panic!("{} should exist: {error}", path.display()))
            .permissions()
            .mode()
            & 0o777
    }

    #[test]
    fn the_directory_is_private_before_the_file_exists() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join("nested/track.kv");
        prepare_directory(&path).expect("the directory is made");

        assert_eq!(mode_of(path.parent().expect("a parent")), PRIVATE_DIR);
        assert!(!path.exists(), "and nothing has created the file yet");
    }

    #[test]
    fn a_tagdata_database_is_recognized() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join("track.kv");
        let _db = tagdata::OpenOptions::new()
            .pagesize(PAGE_SIZE)
            .open(&path)
            .expect("the database opens");

        assert!(is_a_database(&path));
    }

    #[test]
    fn another_tagdata_page_size_is_refused() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join("other-page-size.kv");
        let db = tagdata::OpenOptions::new()
            .pagesize(8192)
            .open(&path)
            .expect("the fixture opens");
        drop(db);

        assert!(!is_a_database(&path));
        assert!(super::super::Store::open_existing(&path, true).is_err());
    }

    #[test]
    fn the_store_left_by_an_older_oslo_is_not_mistaken_for_this_one() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let sqlite = dir.path().join("track.db");
        let mut header = b"SQLite format 3\0".to_vec();
        header.resize(2 * PAGE_SIZE as usize, 0);
        std::fs::write(&sqlite, &header).expect("written");

        assert!(!is_a_database(&sqlite));
    }

    #[test]
    fn invalid_files_are_refused() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let long = 2 * PAGE_SIZE as usize;

        for (name, contents) in [
            ("empty", Vec::new()),
            ("text", b"not a database".to_vec()),
            ("junk", vec![b'x'; long]),
            ("zeros", vec![0; long]),
        ] {
            let path = dir.path().join(name);
            std::fs::write(&path, contents).expect("written");
            assert!(!is_a_database(&path), "{name}");
        }
        assert!(!is_a_database(dir.path()), "a directory at the path");
    }

    #[test]
    fn a_missing_path_can_become_a_database() {
        let dir = tempfile::tempdir().expect("a temp dir");
        assert!(is_a_database(&dir.path().join("not-there-yet.kv")));
    }
}

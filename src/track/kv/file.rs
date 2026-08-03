//! The file itself: whose it is, and whether it is one of ours.
//!
//! # Private, and the ordering jammdb forces
//!
//! `super::super::private` opens with the argument, and it is unchanged: this file is a plaintext
//! record of every command line the shell was told to remember and every directory it stood in, so
//! it is `0600`. What changes is *when*, and the reason is measured rather than assumed.
//!
//! Under turso the rule was "create the file, and its `-wal`, at zero length before the engine is
//! handed the path" — because turso's sidecar did not inherit the database's mode and was born
//! world-readable during `open`. jammdb inverts every part of that:
//!
//! * **There is no sidecar.** One file, no `-wal`, no `-shm`, no lock file. Measured: after a
//!   write and a commit the directory holds exactly one entry. That is a real simplification and
//!   half of `private.rs` goes with it.
//! * **A file that already exists cannot be initialised.** `OpenOptions::open` takes the
//!   `path.exists()` branch and tries to read a meta page out of a zero-length mmap, and it does
//!   not fail — it **panics**, `index out of bounds: the len is 0 but the index is 0`. So creating
//!   the file first, which is exactly what the old rule required, kills the shell.
//! * **A new file is born `0666 & ~umask`.** Measured at `0664` under a `002` umask.
//!
//! So the ordering here is: the *directory* is made `0700` first, then jammdb creates the file
//! inside it, then the file is chmodded `0600` before [`super::Store::open`] returns — and
//! therefore before any caller can possibly have written a command line into it. Nothing secret
//! exists during the window, and during the window the directory is already closed, so the file is
//! unreachable to another account by two independent arguments rather than one.
//!
//! # Nothing but a database is handed to jammdb
//!
//! Measured on 0.11.0, and this is the trap worth stating plainly: `DB::open` **panics** on every
//! file that is not a jammdb database — zero length, a text file, a page of zeros, a database
//! written with a different pagesize. It returns a clean error only when the path is a directory.
//!
//! That is not a hypothetical. `~/.local/share/oslo/track.db` is a *SQLite* file on every machine
//! that has run a released oslo, so a store that opened whatever it found at that path would abort
//! the shell of every existing user on the first upgrade. [`is_a_database`] reads the first meta
//! page and refuses anything else, and [`super::Store`] wraps the open in `catch_unwind` besides,
//! for the corruption that gets past a header check.

use std::fs::Permissions;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

/// The mode the store is kept at.
pub(super) const PRIVATE: u32 = 0o600;

/// The mode the directory holding it is kept at.
///
/// The store's own directory, so this is not oslo tightening somebody else's: `history.db` and the
/// direnv allow list are in here too, and neither is other people's business either.
pub(super) const PRIVATE_DIR: u32 = 0o700;

/// The page size the store is created and opened with.
///
/// Pinned rather than taken from the default, which is the *system* page size. A file written on a
/// 4 KiB-page machine and opened on a 64 KiB-page one — a shared `$HOME`, an aarch64 laptop next
/// to an x86 desktop — would then hit the `assert_eq!` inside jammdb's meta check and panic. A
/// constant is portable; `getpagesize()` is not.
pub(super) const PAGE_SIZE: u64 = 4096;

/// jammdb's on-disk marker, from `db.rs`: `MAGIC_VALUE`, in the meta page's native-endian `u32`.
const MAGIC: u32 = 0x00AB_CDEF;

/// Where that `u32` sits inside page zero: past the page header (`id`, `page_type`, `count`,
/// `overflow`) and past the meta's own `meta_page`. Verified by hexdump against a file jammdb
/// wrote, rather than derived from a `#[repr(C)]` whose padding this module does not control.
const MAGIC_AT: usize = 36;

/// The page type byte, at the offset after the `u64` page id.
const PAGE_TYPE_AT: usize = 8;

/// What page zero of a database says it is.
const TYPE_META: u8 = 0x03;

/// Make the directory the store lives in, and close it to everybody else.
///
/// `None` when it cannot be made: a store that cannot be private is one this shell does without,
/// which is `private.rs`'s rule and is not softened here.
pub(super) fn prepare_directory(path: &Path) -> Option<()> {
    let parent = path.parent()?;
    if !parent.as_os_str().is_empty() {
        std::fs::create_dir_all(parent).ok()?;
        std::fs::set_permissions(parent, Permissions::from_mode(PRIVATE_DIR)).ok()?;
    }
    Some(())
}

/// Tighten the store to its owner. Called the instant the engine has created it, and again on
/// every open so that a file an earlier version left loose is repaired rather than left as found.
pub(super) fn make_private(path: &Path) -> Option<()> {
    std::fs::set_permissions(path, Permissions::from_mode(PRIVATE)).ok()
}

/// Whether the file at `path` is a jammdb database this build can open.
///
/// `true` for a path with nothing at it — that is the file jammdb is *about* to create, and the
/// only case in which it may create one. Everything else has to prove itself, because the
/// alternative is a panic rather than an error.
///
/// Public because [`super::Store::open`] is not the only caller that needs the answer: anything
/// holding a path an *older* oslo wrote has to move that file aside before opening, and asking here
/// is how it knows to. `startup::history_db::migrate` is the one that does.
pub fn is_a_database(path: &Path) -> bool {
    let header = match read_header(path) {
        Ok(Some(header)) => header,
        // Nothing there: jammdb will make one.
        Ok(None) => return true,
        Err(()) => return false,
    };
    header.get(PAGE_TYPE_AT) == Some(&TYPE_META) && magic_of(&header) == Some(MAGIC)
}

/// The first meta page's header, `None` when the file does not exist, `Err` when it exists and is
/// too short to be one.
fn read_header(path: &Path) -> Result<Option<Vec<u8>>, ()> {
    use std::io::Read;

    let mut file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(()),
    };
    // Two meta pages, so a file shorter than that cannot be a database whatever its first bytes
    // say — and a database is never shorter, because `init_file` allocates thirty-two pages.
    match file.metadata() {
        Ok(meta) if meta.is_file() && meta.len() >= 2 * PAGE_SIZE => {}
        _ => return Err(()),
    }
    let mut header = vec![0u8; MAGIC_AT + 4];
    file.read_exact(&mut header).map_err(|_| ())?;
    Ok(Some(header))
}

fn magic_of(header: &[u8]) -> Option<u32> {
    let bytes = header.get(MAGIC_AT..MAGIC_AT + 4)?;
    Some(u32::from_ne_bytes(bytes.try_into().ok()?))
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

    /// The directory is closed before the engine is allowed to make anything inside it, which is
    /// what covers the instant between jammdb creating the file at `0664` and this module
    /// tightening it.
    #[test]
    fn the_directory_is_private_before_the_file_exists() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join("nested/track.kv");
        prepare_directory(&path).expect("the directory is made");

        assert_eq!(mode_of(path.parent().expect("a parent")), PRIVATE_DIR);
        assert!(!path.exists(), "and nothing has created the file yet");
    }

    /// A SQLite file is what every existing oslo has at this path. Handing one to jammdb panics,
    /// so it is refused here — which is the difference between an upgrade that quietly stops
    /// tracking and an upgrade that will not start a shell.
    #[test]
    fn the_store_left_by_an_older_oslo_is_not_mistaken_for_this_one() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let sqlite = dir.path().join("track.db");
        let mut header = b"SQLite format 3\0".to_vec();
        header.resize(2 * PAGE_SIZE as usize, 0);
        std::fs::write(&sqlite, &header).expect("written");

        assert!(!is_a_database(&sqlite));
    }

    /// Every shape of "not a database" that was measured to panic `DB::open`, refused before it
    /// gets there.
    #[test]
    fn nothing_but_a_database_is_offered_to_the_engine() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let long = 2 * PAGE_SIZE as usize;

        let empty = dir.path().join("empty");
        std::fs::write(&empty, b"").expect("written");
        assert!(!is_a_database(&empty), "zero length");

        let text = dir.path().join("text");
        std::fs::write(&text, b"not a database, a text file somebody put here").expect("written");
        assert!(!is_a_database(&text), "too short to be one");

        let junk = dir.path().join("junk");
        std::fs::write(&junk, vec![b'x'; long]).expect("written");
        assert!(!is_a_database(&junk), "long enough, and still not one");

        let zeros = dir.path().join("zeros");
        std::fs::write(&zeros, vec![0u8; long]).expect("written");
        assert!(
            !is_a_database(&zeros),
            "a meta page of nothing is not a meta page"
        );

        assert!(!is_a_database(dir.path()), "a directory at the path");
    }

    /// A path with nothing at it is the one case that passes, because it is the case where jammdb
    /// is allowed to create the file.
    #[test]
    fn a_path_with_nothing_at_it_is_a_store_waiting_to_be_made() {
        let dir = tempfile::tempdir().expect("a temp dir");
        assert!(is_a_database(&dir.path().join("not-there-yet.kv")));
    }
}

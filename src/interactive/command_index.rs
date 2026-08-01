//! The set of runnable command names, cached so that typing does not walk `$PATH`.
//!
//! The hinter used to rebuild this on every keystroke: lock the environment, `read_dir` all of
//! `$PATH` — measured here at 113 directories and 3373 executables — and allocate a `String` per
//! entry, 3.3 ms and 3373 allocations for one character. The set only changes when `$PATH`
//! changes or a directory on it does, so it is cached on exactly those two facts.

use std::collections::HashSet;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

/// How long a cached set is trusted before its directories are re-stat'ed.
///
/// Stat'ing 113 directories is far cheaper than reading them, but it is not free either, and a
/// new executable appearing mid-keystroke is not worth paying for on every character.
const REVALIDATE_AFTER: Duration = Duration::from_millis(500);

/// Bumped by [`invalidate`]; part of the cache key, so a bump discards whatever is cached.
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// Drop the cached command set.
///
/// `hash -r` means "forget where commands live", and a shell that kept completing a command it
/// had just been told to forget would be lying. Called from the `hash` builtin.
pub fn invalidate() {
    GENERATION.fetch_add(1, Ordering::Relaxed);
}

/// Build the cache now, on a thread of its own, so the first Tab does not pay for it.
///
/// Reading 113 directories and 3373 executables costs about 10ms warm and a great deal more from a
/// cold page cache — and it was being paid on the first keystroke that wanted a completion, which
/// is the one moment the shell is being watched. The work is the same; it just happens while you
/// are still reading the prompt.
///
/// Detached on purpose: nothing waits for it. If the first Tab arrives before it finishes, the two
/// meet at the cache's own lock and the second one to arrive uses what the first built — the
/// existing behaviour, at its existing cost, which is the worst this can do.
pub fn warm(path: String) {
    std::thread::spawn(move || {
        let _ = CommandIndex::executables(&path);
    });
}

#[derive(PartialEq, Eq)]
struct Key {
    generation: u64,
    path: String,
    /// One entry per `$PATH` directory: its modification time, or `None` if it does not exist.
    stamps: Vec<Option<SystemTime>>,
}

struct Entry {
    key: Key,
    names: Arc<HashSet<String>>,
    checked_at: Instant,
}

/// A process-wide cache of the executables on `$PATH`.
///
/// Global rather than a field on the helper so that the `hash` builtin — which has no way to
/// reach the line editor — can still invalidate it.
pub struct CommandIndex;

fn cache() -> &'static Mutex<Option<Entry>> {
    static CACHE: OnceLock<Mutex<Option<Entry>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

impl CommandIndex {
    /// Every executable reachable by bare name through `path`.
    ///
    /// The result is shared, not copied: callers filter it by prefix without allocating.
    pub fn executables(path: &str) -> Arc<HashSet<String>> {
        let generation = GENERATION.load(Ordering::Relaxed);
        let mut guard = cache().lock().unwrap();

        if let Some(entry) = guard.as_ref() {
            let fresh = entry.key.generation == generation
                && entry.key.path == path
                && entry.checked_at.elapsed() < REVALIDATE_AFTER;
            if fresh {
                return Arc::clone(&entry.names);
            }
        }

        let key = Key {
            generation,
            path: path.to_string(),
            stamps: stamps(path),
        };

        if let Some(entry) = guard.as_mut()
            && entry.key == key
        {
            // Nothing moved; renew the lease rather than re-reading the directories.
            entry.checked_at = Instant::now();
            return Arc::clone(&entry.names);
        }

        let names = Arc::new(scan(path));
        *guard = Some(Entry {
            key,
            names: Arc::clone(&names),
            checked_at: Instant::now(),
        });
        names
    }

    /// Whether a bare command name resolves to something runnable.
    ///
    /// Names containing `/` are not looked up here: they are paths, and the caller has to stat
    /// them itself.
    pub fn contains(path: &str, name: &str) -> bool {
        !name.contains('/') && Self::executables(path).contains(name)
    }
}

fn stamps(path: &str) -> Vec<Option<SystemTime>> {
    path.split(':')
        .map(|dir| fs::metadata(dir).and_then(|m| m.modified()).ok())
        .collect()
}

fn scan(path: &str) -> HashSet<String> {
    let mut names = HashSet::new();
    for dir in path.split(':') {
        // An empty `$PATH` element means the current directory, per POSIX.
        let dir = if dir.is_empty() { "." } else { dir };
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            if is_executable(&entry) {
                names.insert(entry.file_name().to_string_lossy().into_owned());
            }
        }
    }
    names
}

/// Whether a directory entry is something the shell could actually run.
///
/// The permission bit is checked, not just "is it a file": `$PATH` directories are full of
/// data files, and offering `pkgconfig` as a command is noise.
fn is_executable(entry: &fs::DirEntry) -> bool {
    // `metadata` follows symlinks, which is what we want: a dangling link is not runnable.
    let Ok(meta) = entry.metadata() else {
        return false;
    };
    meta.is_file() && meta.permissions().mode() & 0o111 != 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// The cache is one global slot; two tests using different `$PATH`s would evict each other.
    static SERIAL: Mutex<()> = Mutex::new(());

    fn make_exe(dir: &std::path::Path, name: &str) {
        let p = dir.join(name);
        let mut f = fs::File::create(&p).unwrap();
        f.write_all(b"#!/bin/sh\n").unwrap();
        drop(f);
        fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[test]
    fn finds_executables_and_skips_data_files() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        make_exe(dir.path(), "oslo-test-exe");
        fs::write(dir.path().join("oslo-test-data"), b"x").unwrap();

        let path = dir.path().to_str().unwrap();
        let names = CommandIndex::executables(path);
        assert!(names.contains("oslo-test-exe"));
        assert!(!names.contains("oslo-test-data"));
    }

    #[test]
    fn a_second_lookup_is_served_from_the_cache() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        make_exe(dir.path(), "oslo-cached-exe");
        let path = dir.path().to_str().unwrap();

        let first = CommandIndex::executables(path);
        let second = CommandIndex::executables(path);
        // Same allocation, so no directory was read the second time.
        assert!(Arc::ptr_eq(&first, &second));
    }

    #[test]
    fn invalidate_forces_a_rescan() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_str().unwrap();
        let before = CommandIndex::executables(path);
        make_exe(dir.path(), "oslo-late-exe");
        invalidate();
        let after = CommandIndex::executables(path);
        assert!(!before.contains("oslo-late-exe"));
        assert!(after.contains("oslo-late-exe"));
    }

    #[test]
    fn a_different_path_is_a_different_answer() {
        let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        make_exe(a.path(), "oslo-in-a");
        make_exe(b.path(), "oslo-in-b");

        assert!(CommandIndex::executables(a.path().to_str().unwrap()).contains("oslo-in-a"));
        assert!(CommandIndex::executables(b.path().to_str().unwrap()).contains("oslo-in-b"));
        assert!(!CommandIndex::executables(b.path().to_str().unwrap()).contains("oslo-in-a"));
    }
}

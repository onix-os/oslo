//! Where this binary is, in a form that still works after it has been replaced.
//!
//! # The bug this exists for
//!
//! `std::env::current_exe()` reads `/proc/self/exe` and hands back what it points at. Install a new
//! oslo over the running one — which is what `make install` does, and what a package upgrade does —
//! and the old file is unlinked while the process holds it open. The link then reads:
//!
//! ```text
//! /usr/bin/oslo (deleted)
//! ```
//!
//! …suffix and all. Every caller that *executes* that path gets `ENOENT` and reports
//! `oslo: make: cannot execute`, which is what a running shell did to every `make` after an
//! install. The shell itself is unharmed; only its attempts to start a copy of itself fail.
//!
//! # Why the magic link is the answer
//!
//! `/proc/self/exe` is not an ordinary symlink. The kernel resolves it to the **inode**, so opening
//! or executing it works whether or not the file still has a name — verified: `stat -L` reports the
//! real size on a deleted binary, and `execve` on it succeeds.
//!
//! So a running shell goes on starting the binary it *is*, which is also what the surrounding code
//! wanted: the shell you are typing at is the one whose recipes you mean, and after an upgrade it
//! keeps the build it was started with until you restart it — exactly what `make install` says.

use std::path::PathBuf;

/// The magic link, which resolves to this process's image rather than to a name.
const MAGIC: &str = "/proc/self/exe";

/// This binary, as something that can be executed.
///
/// Prefer this to [`std::env::current_exe`] anywhere the answer is going to be run. For *printing*
/// a path, `current_exe` is better: it names the file a person can look at, and its `(deleted)`
/// suffix is a true and useful thing to see.
pub fn path() -> PathBuf {
    // `exists` follows the link, so this is false only where there is no procfs at all.
    let magic = PathBuf::from(MAGIC);
    if magic.exists() {
        return magic;
    }
    std::env::current_exe().unwrap_or_else(|_| PathBuf::from("oslo"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// It answers something runnable, and on Linux that is the magic link.
    #[test]
    fn the_path_is_executable() {
        let found = path();
        assert!(found.exists(), "{found:?} does not exist");

        let meta = std::fs::metadata(&found).expect("stat");
        assert!(meta.is_file(), "{found:?} is not a file");
        use std::os::unix::fs::PermissionsExt;
        assert!(
            meta.permissions().mode() & 0o111 != 0,
            "{found:?} is not executable"
        );
    }

    /// The same size as the binary the test runner is, which is how we know the link resolved to
    /// this process's image rather than to something else on disk.
    #[test]
    fn it_resolves_to_this_process() {
        let by_link = std::fs::metadata(path()).expect("stat the link").len();
        let by_name = std::env::current_exe()
            .ok()
            .and_then(|path| std::fs::metadata(path).ok())
            .map(|meta| meta.len());
        // `by_name` is `None` for a test binary that has been rebuilt underneath the run, which is
        // the very case this module exists for — so its absence is not a failure.
        if let Some(by_name) = by_name {
            assert_eq!(by_link, by_name);
        }
    }
}

//! Paths and watches for a directory environment.
//!
//! Small, and used from two directions: `oslo.direnv.watch_file`, `watch_dir` and `path_add` in a
//! `.env.lua`, and [`super::Direnv::arrive`], which drains the watch list when a load ends.
//!
//! # Why this is a file of its own
//!
//! It was four functions inside the `.envrc` stdlib, because that is where `watch_file` and
//! `PATH_add` first needed them. The stdlib is gone — oslo does not read `.envrc` any more — and
//! these are the part of it that was never about `.envrc` at all: a `.env.lua` asks to be reloaded
//! when a file changes and puts a directory on `$PATH`, and it did so through here the whole time.

use crate::env::Environment;
use crate::env::lists;
use oslo_base::error::Result;
use std::cell::RefCell;
use std::path::{Path, PathBuf};

thread_local! {
    /// Files this run asked to be watched, beyond the directory file itself.
    ///
    /// Thread-local rather than threaded through every caller, and drained by [`take_watches`]
    /// when the run ends so that one directory's watches cannot become another's.
    static WATCHED: RefCell<Vec<PathBuf>> = const { RefCell::new(Vec::new()) };
}

/// Record a file whose edit should reload the environment.
pub fn watch(path: &Path) {
    let path = path.to_path_buf();
    WATCHED.with(|watched| {
        let mut watched = watched.borrow_mut();
        if !watched.contains(&path) {
            watched.push(path);
        }
    });
}

/// Everything [`watch`] collected, clearing it.
pub fn take_watches() -> Vec<PathBuf> {
    WATCHED.with(|watched| std::mem::take(&mut *watched.borrow_mut()))
}

/// The directory the shell is standing in.
///
/// The process's, not a variable's: `$PWD` is what the shell *published* and a directory file is
/// free to have overwritten it, while every path resolved here has to be relative to where the
/// file actually is.
pub(crate) fn here() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

/// `path` made absolute against `base`, with `~` expanded, and no filesystem access.
///
/// Lexical on purpose: a directory file names paths that do not exist yet — a cache location, a
/// virtualenv it is about to create — and asking the filesystem would answer for the wrong one.
pub fn absolute(path: &str, base: &Path) -> PathBuf {
    let expanded = match path.strip_prefix("~") {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => match std::env::var("HOME") {
            Ok(home) => PathBuf::from(home).join(rest.trim_start_matches('/')),
            Err(_) => PathBuf::from(path),
        },
        _ => PathBuf::from(path),
    };
    if expanded.is_absolute() {
        return tidy(&expanded);
    }
    tidy(&base.join(expanded))
}

/// `.` and `..` resolved textually, so a path that does not exist still comes out clean.
fn tidy(path: &Path) -> PathBuf {
    let mut parts: Vec<std::ffi::OsString> = Vec::new();
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if parts.pop().is_none() {
                    out.push("..");
                }
            }
            std::path::Component::Normal(name) => parts.push(name.to_os_string()),
            other => out.push(other.as_os_str()),
        }
    }
    for part in parts {
        out.push(part);
    }
    out
}

/// Put `dirs` at the front of `name`, absolute, in the order given, each appearing once.
///
/// **Idempotent, which is the whole point.** A directory environment is loaded and reloaded — on an
/// edit, on a nested shell, on `direnv allow` — and appending unconditionally would grow `$PATH` by
/// one entry every time until it was pages long. An entry already present is moved to the front
/// rather than added again.
pub fn prepend_into(env: &mut Environment, name: &str, dirs: &[String]) -> Result<i32> {
    if dirs.is_empty() {
        eprintln!("direnv: path_add: needs a directory");
        return Ok(1);
    }
    lists::prepend(env, name, dirs, &here());
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The lexical rules, on paths that do not exist.
    #[test]
    fn a_relative_path_resolves_against_its_base_without_touching_the_disk() {
        let base = Path::new("/project/sub");
        assert_eq!(absolute("cache", base), PathBuf::from("/project/sub/cache"));
        assert_eq!(absolute("../cache", base), PathBuf::from("/project/cache"));
        assert_eq!(
            absolute("./cache", base),
            PathBuf::from("/project/sub/cache")
        );
        assert_eq!(absolute("/tmp/cache", base), PathBuf::from("/tmp/cache"));
    }

    /// The watch list is per run: a load that took its watches leaves none behind for the next.
    #[test]
    fn watches_are_collected_once_and_drained() {
        let _ = take_watches();
        watch(Path::new("/a"));
        watch(Path::new("/b"));
        watch(Path::new("/a"));
        assert_eq!(
            take_watches(),
            vec![PathBuf::from("/a"), PathBuf::from("/b")],
            "each file once, in the order it was asked for"
        );
        assert!(take_watches().is_empty(), "and the list is now empty");
    }
}

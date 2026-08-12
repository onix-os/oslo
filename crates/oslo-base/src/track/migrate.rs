//! Bringing a flat `<data>/oslo/<profile>.kv` forward into `<data>/oslo/history/<profile>/`.
//!
//! ```text
//! before                          after
//! oslo/claude.kv                  oslo/history/claude/hist.db
//! oslo/claude.kv.lock             oslo/history/claude/hist.lock
//! oslo/claude.model               oslo/history/claude/hist.model
//! ```
//!
//! # Copied, not moved
//!
//! The old files are left exactly where they are. A shell that is already running is running the
//! *old* binary, holding the old store open, and will keep appending to it until it exits — moving
//! the file out from under it would break a session somebody is in the middle of. So this leaves
//! both layouts on disk and lets the old shells die of natural causes.
//!
//! The cost is that lines typed in an old shell after the copy do not reach the new store. That is
//! the honest trade for not disturbing a running session, and it is why the old files are yours to
//! delete rather than something that disappears on its own.
//!
//! # A consistent copy, not `cp`
//!
//! The store may be open and being written to by one of those old shells. `Track::backup_to` takes
//! the same snapshot `oslo history backup` does, so what lands is a database rather than whatever
//! bytes happened to be on disk halfway through somebody else's transaction. The model is a plain
//! file written by rename, so a plain copy of it is already atomic.
//!
//! # Once, and never again
//!
//! The question "has this run?" is answered by the new store existing. No stamp file, no flag: if
//! `hist.db` is there, this profile has moved, and a second copy would overwrite a store that has
//! since been written to.

use super::profile;
use std::path::{Path, PathBuf};

/// Copy every flat-layout profile into the new one, for the profiles that have not moved yet.
///
/// Answers the names it brought forward. Silent when there is nothing to do, which is every run
/// after the first.
///
/// **Called only where a store is about to be opened**, never from `main`. It was in `main` for one
/// build, and that is a mistake worth recording: every `sh -c` — a maintainer script, a `Makefile`
/// rule, a sandboxed build step — then tried to create directories and copy megabytes under
/// `$HOME` before running its command. A shell that cannot write there is a perfectly good shell
/// for running `echo`, and this turned that into a shell that could not start.
///
/// Once per process, because both readers may ask.
pub fn from_flat_layout() -> Brought {
    static DONE: std::sync::OnceLock<Brought> = std::sync::OnceLock::new();
    DONE.get_or_init(|| {
        let xdg = std::env::var("XDG_DATA_HOME").ok();
        let home = std::env::var("HOME").ok();
        from_flat_layout_in(xdg.as_deref(), home.as_deref())
    })
    .clone()
}

/// What one pass did.
///
/// **`failed` is not a detail.** A store that will not open is left behind silently otherwise, and
/// the profile simply appears empty in the new layout — a history somebody thinks they still have.
/// Both this machine's `claude` and `codex` stores turned out to be unreadable when this was
/// written, which is exactly how the case was found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Brought {
    pub moved: Vec<String>,
    pub failed: Vec<String>,
}

pub fn from_flat_layout_in(xdg_data: Option<&str>, home: Option<&str>) -> Brought {
    let Some(root) = profile::store_dir(xdg_data, home) else {
        return Brought::default();
    };
    let mut brought = Brought::default();
    for name in flat_profiles(&root) {
        let Some(directory) = profile::profile_dir(xdg_data, home, &name) else {
            continue;
        };
        if directory.join("hist.db").exists() {
            continue;
        }
        match copy_one(&root, &name, &directory) {
            Some(()) => brought.moved.push(name),
            None => brought.failed.push(name),
        }
    }
    brought.moved.sort();
    brought.failed.sort();
    brought
}

/// The profiles the old layout has: every `<name>.kv` directly under `<data>/oslo/`.
fn flat_profiles(root: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension()? != "kv" {
                return None;
            }
            let name = path.file_stem()?.to_str()?.to_string();
            // `available()` never showed a profile whose name it would refuse, and neither does
            // this: a file called `something else.kv` is not a profile that was ever reachable.
            profile::valid(&name).then_some(name)
        })
        .collect()
}

fn copy_one(root: &Path, name: &str, directory: &Path) -> Option<()> {
    std::fs::create_dir_all(directory).ok()?;
    // The same mode the store's own directory gets: a history is as private as what is in it.
    let _ = std::fs::set_permissions(directory, private_dir());

    let old_store = root.join(format!("{name}.kv"));
    let new_store = directory.join("hist.db");
    if !copy_store(&old_store, &new_store) {
        // Nothing landed, so leave no directory suggesting a profile that is not there. The lock
        // goes with it: opening a store creates one before it discovers it cannot read the file,
        // and a directory holding nothing but a lock is what stops `remove_dir` here.
        let _ = std::fs::remove_file(directory.join("hist.lock"));
        let _ = std::fs::remove_dir(directory);
        return None;
    }

    // The model is optional — a profile that has never had one is not a failed migration, and a
    // model that fails to copy is a model that will be learnt again.
    let old_model = root.join(format!("{name}.model"));
    if old_model.is_file() {
        let _ = std::fs::copy(&old_model, directory.join("hist.model"));
    }
    Some(())
}

/// Get `old` to `new` as a database, whichever of the two ways works.
///
/// **The snapshot first, the byte copy second.** A store that opens is snapshotted the way
/// `oslo history backup` does it, which is safe while another shell is writing to it — and the
/// stores most likely to have a shell in them are exactly the ones written by this version.
///
/// A store on an *older schema* cannot be opened that way at all: `open_existing` refuses any
/// version but the current one, and only `Track::open` migrates — which would write to the file the
/// old binary is still using. So those are copied byte for byte and the **copy** is migrated, which
/// upgrades the new store and leaves the original exactly as it was.
fn copy_store(old: &Path, new: &Path) -> bool {
    if let Ok(store) = super::Track::open_existing(old, true)
        && store.backup_to(new).is_ok()
    {
        return true;
    }
    if std::fs::copy(old, new).is_err() {
        return false;
    }
    // Opening migrates it forward, and is also the check that what was copied is a database at all.
    // A copy that will not open is removed rather than left to be found later as an empty history.
    if super::Track::open(new).is_some() {
        return true;
    }
    let _ = std::fs::remove_file(new);
    false
}

fn private_dir() -> std::fs::Permissions {
    use std::os::unix::fs::PermissionsExt;
    std::fs::Permissions::from_mode(0o700)
}

/// Where the old layout kept `name`, for a caller that wants to say so.
pub fn legacy_store_of(name: &str) -> Option<PathBuf> {
    profile::legacy_path(
        std::env::var("XDG_DATA_HOME").ok().as_deref(),
        std::env::var("HOME").ok().as_deref(),
        name,
        "kv",
    )
}

#[cfg(test)]
#[path = "migrate/tests.rs"]
mod tests;

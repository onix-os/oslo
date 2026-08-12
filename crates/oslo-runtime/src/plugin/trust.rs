//! Whether a plugin's files are the ones you allowed.
//!
//! **The same shape `.envrc` already uses**, and for the same reason: installing is a decision about
//! *code*, and code that changed is a decision nobody made. `oslo plugin install` records what the
//! files hashed to; loading recomputes and compares; `oslo plugin allow` records the new hash after
//! you have looked at what changed.
//!
//! A `git pull` inside a plugin therefore stops it loading until you say so, which is the whole
//! point — an update is somebody else's new code arriving on your machine.
//!
//! # What is hashed
//!
//! Every `.lua` file in the plugin's directory, in path order, each preceded by its relative path
//! and its length. The path is in the hash so that renaming a file changes it; the length is there
//! so that no concatenation of two files can hash to the same thing as a different pair.
//!
//! Only `.lua`, because only Lua is loaded. A plugin's README changing is not a change to what it
//! will do, and hashing it would make every documentation edit a refusal to run.

use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// What the plugin at `directory` hashes to now.
pub fn hash_of(directory: &Path) -> Result<String, String> {
    let mut files = lua_files(directory)?;
    // Path order, so the hash does not depend on what the filesystem felt like returning.
    files.sort();
    if files.is_empty() {
        return Err(format!("{}: no Lua in it at all", directory.display()));
    }

    let mut hasher = Sha256::new();
    for path in &files {
        let relative = path.strip_prefix(directory).unwrap_or(path);
        let bytes = std::fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?;
        hasher.update(relative.to_string_lossy().as_bytes());
        hasher.update(b"\n");
        hasher.update(bytes.len().to_le_bytes());
        hasher.update(&bytes);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

/// Every `.lua` file under `directory`, at any depth.
fn lua_files(directory: &Path) -> Result<Vec<PathBuf>, String> {
    let mut found = Vec::new();
    let mut stack = vec![directory.to_path_buf()];
    while let Some(here) = stack.pop() {
        let entries =
            std::fs::read_dir(&here).map_err(|error| format!("{}: {error}", here.display()))?;
        for entry in entries.flatten() {
            let path = entry.path();
            // **Symlinks are not followed.** A plugin that links to a file outside its own
            // directory would otherwise be hashed over something the install never saw, and could
            // change without changing the plugin.
            let kind = entry.file_type().map_err(|error| error.to_string())?;
            if kind.is_symlink() {
                continue;
            }
            if kind.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|kind| kind == "lua") {
                found.push(path);
            }
        }
    }
    Ok(found)
}

/// Is what is on disk what was allowed?
pub fn unchanged(directory: &Path, allowed: &str) -> Result<bool, String> {
    Ok(hash_of(directory)? == allowed)
}

#[cfg(test)]
#[path = "trust/tests.rs"]
mod tests;

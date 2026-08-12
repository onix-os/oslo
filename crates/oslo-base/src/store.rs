//! A database that belongs to a config or a plugin, rather than to oslo.
//!
//! The engine, the file handling and the transactions are [`crate::track::kv`]'s — this adds only
//! the two things a caller from Lua must not decide for itself: **where the file goes**, and **what
//! may go in it**.
//!
//! # A name, never a path
//!
//! [`crate::store::open`] takes `"notes"`, not `~/.local/share/oslo/plugins/notes.kv`. A path would
//! mean a plugin could open another plugin's database by naming it, or oslo's own history by naming
//! that, or `/etc/shadow` by naming that and finding out what happens. A name maps to exactly one
//! file in one directory, and a name that could escape it is refused before anything is opened.
//!
//! # One bucket, one file
//!
//! Everything lives in [`crate::track::kv::Tree::Plugin`]. There is nothing to keep apart inside one
//! of these files because one caller owns the whole file — which is also what makes uninstalling a
//! plugin an `rm` rather than a migration.

use crate::track::kv::{Span, Store, Tree};
use std::path::PathBuf;

/// The largest value one key may hold: 1 MiB.
///
/// A ceiling rather than a limit anybody should reach. Values arrive from Lua, where a mistaken
/// `db:set(k, huge)` is one line, and a database is not the place to discover that the shell will
/// allocate whatever it is handed.
pub const MAX_VALUE: usize = 1024 * 1024;

/// The longest key: enough for a path or a URL, short of a document.
pub const MAX_KEY: usize = 1024;

/// Where these live: `$XDG_DATA_HOME/oslo/plugins`, or `~/.local/share/oslo/plugins`.
///
/// Beside the profile's history and the model, because this is state the user accumulates and would
/// miss — not a cache. `None` when neither variable is knowable, which for a database is an error
/// rather than the silent no-op tracking is allowed to be.
pub fn directory() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .or_else(|| {
            std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share"))
        })?;
    Some(base.join("oslo/plugins"))
}

/// Is `name` one this may open?
///
/// Deliberately narrower than a filename: no separator, no leading dot, nothing that means
/// something to a shell or a path parser. The check is on the name as given, so there is no case
/// where a name is transformed into a legal one and quietly opens a file the caller did not ask
/// for.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .enumerate()
            .all(|(at, byte)| byte.is_ascii_alphanumeric() || (at > 0 && b"._-".contains(&byte)))
}

/// The file `name` maps to.
pub fn path_of(name: &str) -> Option<PathBuf> {
    valid_name(name)
        .then(directory)?
        .map(|directory| directory.join(format!("{name}.kv")))
}

/// Open the database called `name`, creating it if it is not there.
///
/// Private on creation — mode `0600` on the file and `0700` on the directory — because a config's
/// database is as likely to hold a token as anything oslo writes itself.
pub fn open(name: &str) -> Result<Store, String> {
    if !valid_name(name) {
        return Err(format!(
            "{name:?} is not a database name: letters, digits, and `.` `_` `-` after the first"
        ));
    }
    let path = path_of(name).ok_or_else(|| {
        "no $XDG_DATA_HOME and no $HOME, so there is nowhere to keep a database".to_string()
    })?;
    Store::open(&path).ok_or_else(|| format!("{}: cannot be opened", path.display()))
}

/// One key's value.
pub fn get(store: &Store, key: &str) -> Option<Vec<u8>> {
    store.read(|reader| reader.get(Tree::Plugin, key.as_bytes()))
}

/// Whether a key is there at all, which `get` cannot answer for an empty value.
pub fn has(store: &Store, key: &str) -> bool {
    store.read(|reader| reader.has(Tree::Plugin, key.as_bytes()).then_some(true)) == Some(true)
}

/// Write one key.
pub fn set(store: &Store, key: &str, value: &[u8]) -> Result<(), String> {
    check(key, Some(value))?;
    store
        .write(|writer| writer.put(Tree::Plugin, key.as_bytes().to_vec(), value.to_vec()))
        .ok_or_else(|| format!("{}: the write did not commit", store.path().display()))
}

/// Remove one key. Answers whether it was there.
pub fn delete(store: &Store, key: &str) -> bool {
    store
        .write(|writer| Some(writer.delete(Tree::Plugin, key.as_bytes())))
        .unwrap_or(false)
}

/// Every key beginning with `prefix`, in bytewise order.
pub fn keys(store: &Store, prefix: &str) -> Vec<String> {
    let mut found = Vec::new();
    store.read(|reader| {
        reader.scan(
            Tree::Plugin,
            &Span::prefix(prefix.as_bytes().to_vec()),
            |key, _| {
                // Lossy, because a key written from Lua is a Lua string and those are bytes. A key
                // that is not text is still listed rather than silently skipped, so a caller can see
                // that it is there.
                found.push(String::from_utf8_lossy(key).into_owned());
                crate::track::kv::Walk::On
            },
        );
        Some(())
    });
    found
}

/// What a key and value must satisfy before a transaction is opened.
fn check(key: &str, value: Option<&[u8]>) -> Result<(), String> {
    if key.is_empty() {
        return Err("a key cannot be empty".to_string());
    }
    if key.len() > MAX_KEY {
        return Err(format!(
            "key is {} bytes; the limit is {MAX_KEY}",
            key.len()
        ));
    }
    match value {
        Some(value) if value.len() > MAX_VALUE => Err(format!(
            "value is {} bytes; the limit is {MAX_VALUE}",
            value.len()
        )),
        _ => Ok(()),
    }
}

#[cfg(test)]
#[path = "store/tests.rs"]
mod tests;

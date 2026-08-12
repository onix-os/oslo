//! The one file startup reads: every installed plugin's declarations, flattened.
//!
//! # Why a generated index and not just the manifests
//!
//! A manifest is the right thing to *write*. It is the wrong thing to read ten of at startup: each
//! is a parse and an evaluation, to learn a list of names. `oslo plugin install` therefore flattens
//! every manifest into this one file, and a session reads it alone — one open, one parse, no Lua.
//!
//! **A stale index is a performance bug, never a correctness one.** It is rebuilt whenever a
//! manifest is newer than it, so a plugin edited by hand still works; what it buys in the ordinary
//! case is that nothing has to be read to know that nothing changed.
//!
//! # What is in it
//!
//! ```json
//! { "version": 1,
//!   "plugins": [ { "name": "notes", "entry": "init.lua",
//!                  "builtins": ["note"], "tools": ["notes"],
//!                  "hash": "…" } ] }
//! ```
//!
//! `hash` is the trust decision: what the plugin's files hashed to when you allowed it. See
//! [`super::trust`].

use super::manifest::Manifest;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// The schema this code writes and understands.
const VERSION: u64 = 1;

/// One installed plugin, as the index records it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    pub name: String,
    pub entry: String,
    pub builtins: Vec<String>,
    pub tools: Vec<String>,
    /// What the plugin's files hashed to when it was allowed.
    pub hash: String,
    /// The oldest oslo it will run on, as the manifest wrote it.
    ///
    /// Carried here so the load-time check costs nothing: re-reading the manifest to learn it would
    /// undo the whole reason the index exists.
    pub requires: Option<String>,
}

impl Installed {
    /// The directory this plugin lives in.
    pub fn directory(&self) -> Option<PathBuf> {
        Some(super::directory()?.join(&self.name))
    }

    /// Every name it reserves.
    pub fn names(&self) -> impl Iterator<Item = &String> {
        self.builtins.iter().chain(self.tools.iter())
    }

    pub fn of(manifest: &Manifest, hash: String) -> Installed {
        Installed {
            name: manifest.name.clone(),
            entry: manifest.entry.clone(),
            builtins: manifest.builtins.clone(),
            tools: manifest.tools.clone(),
            hash,
            requires: manifest.requires.clone(),
        }
    }
}

/// Where the index lives.
pub fn path() -> Option<PathBuf> {
    Some(super::directory()?.join("index.json"))
}

/// Every installed plugin, or nothing at all.
///
/// **Never an error.** A missing index is a shell with no plugins, which is the ordinary case; a
/// corrupt one is a shell with no plugins and a complaint, because refusing to start over a file
/// that is only ever generated would be a poor trade.
pub fn read() -> Vec<Installed> {
    let Some(path) = path() else {
        return Vec::new();
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    match parse(&text) {
        Ok(found) => found,
        Err(message) => {
            eprintln!(
                "oslo: {}: {message}; run `oslo plugin list`",
                path.display()
            );
            Vec::new()
        }
    }
}

/// Read the index's text, separated from the file so it can be tested without one.
pub fn parse(text: &str) -> Result<Vec<Installed>, String> {
    let document: Value = serde_json::from_str(text).map_err(|error| error.to_string())?;
    match document.get("version").and_then(Value::as_u64) {
        Some(VERSION) => {}
        Some(other) => return Err(format!("index version {other} is not one this oslo writes")),
        None => return Err("no version in the index".to_string()),
    }
    let mut found = Vec::new();
    for entry in document
        .get("plugins")
        .and_then(Value::as_array)
        .unwrap_or(&Vec::new())
    {
        let text_of = |field: &str| entry.get(field).and_then(Value::as_str).map(str::to_string);
        let list_of = |field: &str| -> Vec<String> {
            entry
                .get(field)
                .and_then(Value::as_array)
                .map(|values| {
                    values
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default()
        };
        let (Some(name), Some(hash)) = (text_of("name"), text_of("hash")) else {
            return Err("a plugin entry has no name or no hash".to_string());
        };
        found.push(Installed {
            name,
            entry: text_of("entry").unwrap_or_else(|| "init.lua".to_string()),
            builtins: list_of("builtins"),
            tools: list_of("tools"),
            hash,
            requires: text_of("requires"),
        });
    }
    Ok(found)
}

/// Write the index, replacing whatever was there.
pub fn write(entries: &[Installed]) -> Result<(), String> {
    let path = path().ok_or_else(|| "nowhere to keep an index".to_string())?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("{}: {error}", parent.display()))?;
    }
    let document = json!({
        "version": VERSION,
        "plugins": entries.iter().map(|installed| json!({
            "name": installed.name,
            "entry": installed.entry,
            "builtins": installed.builtins,
            "tools": installed.tools,
            "hash": installed.hash,
            "requires": installed.requires,
        })).collect::<Vec<_>>(),
    });
    let text = serde_json::to_string_pretty(&document).map_err(|error| error.to_string())?;
    std::fs::write(&path, text).map_err(|error| format!("{}: {error}", path.display()))
}

/// Is the index older than any plugin's manifest?
///
/// The cheap half of "is this still true": one `stat` per plugin directory, against one for the
/// index. Editing a manifest by hand is a thing people do, and a plugin that stopped working until
/// it was reinstalled would be blamed on the shell.
pub fn is_stale(entries: &[Installed]) -> bool {
    let Some(index) = path() else {
        return false;
    };
    let Some(written) = modified(&index) else {
        return true;
    };
    entries.iter().any(|installed| {
        installed
            .directory()
            .map(|directory| directory.join(super::manifest::FILE))
            .and_then(|manifest| modified(&manifest))
            .is_none_or(|changed| changed > written)
    })
}

fn modified(path: &Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(path).ok()?.modified().ok()
}

#[cfg(test)]
#[path = "index/tests.rs"]
mod tests;

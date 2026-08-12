//! The small named things you accumulate: aliases, abbreviations, functions and scripts.
//!
//! ```text
//! ~/.local/share/oslo/macros/macros.db         the four kinds, keyed `<kind>/<name>`
//! ~/.local/share/oslo/macros/macros.snapshot   what a starting shell actually reads
//! ```
//!
//! # One store, not one per profile
//!
//! History is per profile because an agent's commands must not pollute the ranking of yours.
//! Aliases are the opposite: they are *your tooling*, and adding one in this shell and not finding
//! it in the next is the whole of the surprise. A profile changes what the shell **remembers**, not
//! what it **knows how to do**.
//!
//! # Why there is a snapshot as well as a database
//!
//! Measured, because the answer decided the design:
//!
//! ```text
//! oslo -c true, static musl            842 µs      (dash: 622 µs)
//! opening this database                2.61 ms     three times a whole shell start
//! reading 60 rows out of it            1.13 ms
//! reading 60 aliases from a flat file  3.6 µs
//! ```
//!
//! An interactive shell already pays 2.6 ms for the tracking store; a second one would double that
//! to answer a question a `read(2)` answers in microseconds. So [`snapshot::write`] is called by
//! every mutation and [`snapshot::read`] is what a starting shell reads.
//!
//! **The database stays the single source of truth.** The snapshot is a cache: delete it and the
//! next `oslo macros` command writes it again, and a shell that finds none simply has no aliases
//! until then rather than reading a database on the startup path.
//!
//! # Only two kinds are in the snapshot
//!
//! An alias must be in hand before the first line is parsed, and an abbreviation before the first
//! keystroke. A function or a script is looked up **after** the `$PATH` search has already failed —
//! the rule `exec::simple::autoload` states — so it costs a database open only on a line that was
//! going to fail anyway, and nothing at startup.

pub mod snapshot;

pub use crate::track::kv::Store;
use std::path::PathBuf;

/// What a stored name is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    /// A word replaced before the line is parsed.
    Alias,
    /// A word expanded into the buffer as you type it.
    Abbrev,
    /// A shell or Lua function, found after `$PATH`.
    Func,
    /// A whole program with a shebang, run from memory. See `oslo_shell::exec::stored`.
    Script,
}

impl Kind {
    pub fn word(self) -> &'static str {
        match self {
            Kind::Alias => "alias",
            Kind::Abbrev => "abbrev",
            Kind::Func => "func",
            Kind::Script => "script",
        }
    }

    pub fn named(word: &str) -> Option<Kind> {
        match word {
            "alias" => Some(Kind::Alias),
            "abbrev" | "abbr" => Some(Kind::Abbrev),
            "func" | "function" => Some(Kind::Func),
            "script" => Some(Kind::Script),
            _ => None,
        }
    }

    /// Whether a starting shell needs this before it can do anything.
    pub fn wanted_at_startup(self) -> bool {
        matches!(self, Kind::Alias | Kind::Abbrev)
    }

    /// What a temporary file holding one of these should be called, so an editor colours it.
    ///
    /// A script says what it is in its own first line, so its extension comes from the shebang
    /// rather than from here.
    pub fn extension(self, body: &str) -> &'static str {
        match self {
            Kind::Alias | Kind::Abbrev => "sh",
            Kind::Func => {
                if body.trim_start().starts_with("--") || body.contains("local ") {
                    "lua"
                } else {
                    "sh"
                }
            }
            Kind::Script => match shebang_interpreter(body) {
                Some(interp) if interp.contains("python") => "py",
                Some(interp) if interp.contains("lua") => "lua",
                Some(interp) if interp.contains("perl") => "pl",
                Some(interp) if interp.contains("node") => "js",
                _ => "sh",
            },
        }
    }
}

/// One stored thing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub kind: Kind,
    pub name: String,
    pub body: String,
}

/// The interpreter a script's shebang names, if it has one.
///
/// `#!/usr/bin/env python3` answers `python3` rather than `env`, because what the caller wants to
/// know is the *language*, and `env` is how a shebang finds it rather than what it is.
pub fn shebang_interpreter(body: &str) -> Option<String> {
    let first = body.lines().next()?.trim_end();
    let rest = first.strip_prefix("#!")?.trim();
    let mut words = rest.split_whitespace();
    let program = words.next()?;
    let base = program.rsplit('/').next().unwrap_or(program);
    if base == "env" {
        // `env -S`, `env VAR=x prog` — take the first word that is not a switch or an assignment.
        return words
            .find(|word| !word.starts_with('-') && !word.contains('='))
            .map(|word| word.rsplit('/').next().unwrap_or(word).to_string());
    }
    Some(base.to_string())
}

/// Whether `name` is one this may store.
///
/// The same rule the shell has for a function or alias name, and for the same reason: it becomes a
/// command word, so anything that could be an operator or a separator is not a name.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && !name.starts_with('-')
        && name.chars().all(|c| {
            c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | '+' | ':' | '@' | ',' | '%')
        })
}

/// `$XDG_DATA_HOME/oslo/macros`, or `~/.local/share/oslo/macros`.
pub fn directory() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .or_else(|| {
            std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share"))
        })?;
    Some(base.join("oslo/macros"))
}

pub fn database() -> Option<PathBuf> {
    Some(directory()?.join("macros.db"))
}

pub fn snapshot() -> Option<PathBuf> {
    Some(directory()?.join("macros.snapshot"))
}

/// Open the database, creating it if it is not there.
pub fn open() -> Result<Store, String> {
    let path = database().ok_or_else(|| {
        "no $XDG_DATA_HOME and no $HOME, so there is nowhere to keep macros".to_string()
    })?;
    Store::open(&path).ok_or_else(|| format!("{}: cannot be opened", path.display()))
}

fn key(kind: Kind, name: &str) -> String {
    format!("{}/{name}", kind.word())
}

/// Store one, replacing whatever had that name and kind.
pub fn put(store: &Store, entry: &Entry) -> Result<(), String> {
    if !valid_name(&entry.name) {
        return Err(format!("{:?} is not a name this can store", entry.name));
    }
    crate::store::set(store, &key(entry.kind, &entry.name), entry.body.as_bytes())
}

/// Read one back.
pub fn get(store: &Store, kind: Kind, name: &str) -> Option<Entry> {
    let body = crate::store::get(store, &key(kind, name))?;
    Some(Entry {
        kind,
        name: name.to_string(),
        body: String::from_utf8_lossy(&body).into_owned(),
    })
}

/// Every kind that has something under `name`.
///
/// A name can be an alias *and* a script, which is a mistake rather than a feature — and the way to
/// report it is to be able to see it.
pub fn kinds_of(store: &Store, name: &str) -> Vec<Kind> {
    [Kind::Alias, Kind::Abbrev, Kind::Func, Kind::Script]
        .into_iter()
        .filter(|kind| crate::store::has(store, &key(*kind, name)))
        .collect()
}

/// Remove one. Answers whether it was there.
pub fn remove(store: &Store, kind: Kind, name: &str) -> bool {
    crate::store::delete(store, &key(kind, name))
}

/// Everything stored, sorted by kind then name.
pub fn all(store: &Store) -> Vec<Entry> {
    let mut found = Vec::new();
    for kind in [Kind::Alias, Kind::Abbrev, Kind::Func, Kind::Script] {
        let prefix = format!("{}/", kind.word());
        for full in crate::store::keys(store, &prefix) {
            let Some(name) = full.strip_prefix(&prefix) else {
                continue;
            };
            if let Some(entry) = get(store, kind, name) {
                found.push(entry);
            }
        }
    }
    found.sort_by(|a, b| a.kind.cmp(&b.kind).then_with(|| a.name.cmp(&b.name)));
    found
}

/// Store one and keep the snapshot in step, which is the only way callers should write.
///
/// Separate from [`put`] so that a caller writing a batch pays for one snapshot rather than one per
/// entry — and so that forgetting to rewrite it is a thing you have to do deliberately.
pub fn put_and_publish(store: &Store, entry: &Entry) -> Result<(), String> {
    put(store, entry)?;
    publish(store)
}

/// The same, for a removal.
pub fn remove_and_publish(store: &Store, kind: Kind, name: &str) -> Result<bool, String> {
    let gone = remove(store, kind, name);
    publish(store)?;
    Ok(gone)
}

/// Rewrite the snapshot from what the database now says.
pub fn publish(store: &Store) -> Result<(), String> {
    snapshot::write(&all(store))
}

#[cfg(test)]
#[path = "macros/tests.rs"]
mod tests;

//! The flat file a starting shell reads instead of the database.
//!
//! One line per entry, tab-separated: `kind \t name \t body`. A newline in a body is written `\n`,
//! because a line-oriented file whose records contain raw newlines is not line-oriented — the same
//! escape, for the same reason, as the `$HISTFILE` writer.
//!
//! **Only the kinds a shell needs before it can do anything.** A function or a script is found after
//! `$PATH` has already failed, so it is read from the database at the moment it is called and has no
//! business in a file that is read on every interactive start.
//!
//! # It is a cache, and behaves like one
//!
//! Unreadable, half-written, from a future version with a fourth column — all the same answer: no
//! aliases, and the next `oslo aliases` command writes a good one. A shell that refused to start
//! because a *cache* was malformed would be a worse shell than one that starts without your aliases.

use super::{Entry, Kind};
use std::io::Write;

/// Write the snapshot for `entries`, replacing whatever was there.
///
/// Through a temporary file and a rename, so a shell reading it while `oslo aliases add` runs sees
/// the old file or the new one and never half of either.
pub fn write(entries: &[Entry]) -> Result<(), String> {
    let Some(path) = super::snapshot() else {
        return Err("nowhere to write the snapshot".to_string());
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    let mut text = String::new();
    for entry in entries.iter().filter(|e| e.kind.wanted_at_startup()) {
        text.push_str(entry.kind.word());
        text.push('\t');
        text.push_str(&entry.name);
        text.push('\t');
        text.push_str(&escape(&entry.body));
        text.push('\n');
    }

    let scratch = path.with_extension("snapshot.new");
    let mut file = std::fs::File::create(&scratch)
        .map_err(|e| format!("{}: {}", scratch.display(), crate::error::reason(&e)))?;
    // Private, like everything else oslo keeps: an alias can name a host, a path or a token.
    let _ = file.set_permissions(std::os::unix::fs::PermissionsExt::from_mode(0o600));
    file.write_all(text.as_bytes())
        .map_err(|e| format!("{}: {}", scratch.display(), crate::error::reason(&e)))?;
    drop(file);
    std::fs::rename(&scratch, &path)
        .map_err(|e| format!("{}: {}", path.display(), crate::error::reason(&e)))
}

/// What the snapshot says, or nothing at all.
///
/// Never an error: see the note above about it being a cache.
pub fn read() -> Vec<Entry> {
    let Some(path) = super::snapshot() else {
        return Vec::new();
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    text.lines().filter_map(one).collect()
}

fn one(line: &str) -> Option<Entry> {
    let mut fields = line.splitn(3, '\t');
    let kind = Kind::named(fields.next()?)?;
    let name = fields.next()?;
    let body = fields.next()?;
    // A row for a kind that does not belong here is a snapshot written by something else, or by a
    // version that changed its mind. Skipped rather than trusted.
    if !kind.wanted_at_startup() || !super::valid_name(name) {
        return None;
    }
    Some(Entry {
        kind,
        name: name.to_string(),
        body: unescape(body),
    })
}

/// Forget the file. The database is untouched, so the next write brings it back.
pub fn forget() {
    if let Some(path) = super::snapshot() {
        let _ = std::fs::remove_file(path);
    }
}

fn escape(body: &str) -> String {
    body.replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace('\t', "\\t")
}

fn unescape(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let mut chars = body.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// The escaping, both ways, for a test that has no filesystem to round-trip through.
#[cfg(test)]
pub(super) fn round_trip_for_test(body: &str) -> String {
    unescape(&escape(body))
}

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
//! aliases, and the next `oslo macros` command writes a good one. A shell that refused to start
//! because a *cache* was malformed would be a worse shell than one that starts without your aliases.

use super::{Entry, Kind};
use std::io::Write;

/// Write the snapshot for `entries`, replacing whatever was there.
///
/// Through a temporary file and a rename, so a shell reading it while `oslo macros add` runs sees
/// the old file or the new one and never half of either.
pub fn write(entries: &[Entry]) -> Result<(), String> {
    let path = super::snapshot().ok_or_else(|| "nowhere to write the snapshot".to_string())?;
    write_to(&path, entries)
}

/// The same file, written somewhere else — see [`super::live::elsewhere`], which is the same rows
/// from a different source and has no business inventing a second format for them.
pub fn write_to(path: &std::path::Path, entries: &[Entry]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    let mut text = String::new();
    // **Only what is on.** A macro turned off is still in the database — that is what makes it
    // different from a removed one — but the snapshot is a list of what a shell should do, and a
    // shell filtering it again would be reading a field it has no other use for.
    let wanted = entries
        .iter()
        .filter(|e| e.kind.wanted_at_startup() && e.active);
    for entry in wanted {
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
    std::fs::rename(&scratch, path)
        .map_err(|e| format!("{}: {}", path.display(), crate::error::reason(&e)))
}

/// What the snapshot says, or nothing at all.
///
/// Never an error: see the note above about it being a cache.
pub fn read() -> Vec<Entry> {
    match super::snapshot() {
        Some(path) => read_from(&path),
        None => Vec::new(),
    }
}

pub fn read_from(path: &std::path::Path) -> Vec<Entry> {
    let Ok(text) = std::fs::read_to_string(path) else {
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
    // **The fields a shell does not need are not in the file.** When it was created and what it is
    // tagged belong to the manager, and a row that reached this file is on by definition.
    //
    // `created` is **0, not now**: this file has no timestamp in it, and stamping the moment it was
    // read would have the manager report every alias your config defines as made seconds ago. Zero
    // is what `ago` draws as `—`, which is the truthful answer.
    Some(Entry {
        created: 0,
        ..Entry::new(kind, name, &unescape(body))
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

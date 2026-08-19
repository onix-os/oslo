//! The `@name` directory table, in two layers.
//!
//! **Declared** — `oslo.dirs` in a config, replaced whole on every start — and **marked**, which is
//! a file you add to by typing `mark` in a directory and which survives the session. A lookup asks
//! the declared table first: a name a config states outright is a decision, and a mark made in
//! passing should not quietly take it over.
//!
//! The sigil is only ever *read*: `@name` expands as an argument, and never as the first word of a
//! line, whose leading position is reserved.
//!
//! It lives here because the expansion that turns `@work` into a path, the builtin that writes a
//! mark, and the completion that offers the names are in three crates that cannot see each other.
//!
//! Not to be confused with `oslo_ui::marks`, which is the terminal's `OSC 133` prompt marks — a
//! different thing entirely that shares an English word.

use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::RwLock;

static NAMED: RwLock<Option<HashMap<String, String>>> = RwLock::new(None);

/// Register the `@name` table a config declared, replacing whatever was there.
///
/// A leading `~` is resolved here, so no caller has to know about tildes to answer what `@work`
/// means.
pub fn set_named_dirs(dirs: HashMap<String, String>) {
    let resolved = dirs
        .into_iter()
        .map(|(name, path)| (name, expand_tilde(&path)))
        .collect();
    if let Ok(mut slot) = NAMED.write() {
        *slot = Some(resolved);
    }
}

/// A leading `~`, so a config may write `~/data/code` and a mark may store a real path.
fn expand_tilde(path: &str) -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    match path.strip_prefix('~') {
        Some(rest) if !home.is_empty() && (rest.is_empty() || rest.starts_with('/')) => {
            format!("{home}{rest}")
        }
        _ => path.to_string(),
    }
}

/// The path `@name` stands for, if anything registered or marked one.
///
/// Declared first, then marked — see the note at the top of the file.
pub fn named_dir(name: &str) -> Option<String> {
    if let Ok(slot) = NAMED.read()
        && let Some(path) = slot.as_ref().and_then(|table| table.get(name))
    {
        return Some(path.clone());
    }
    marks()
        .into_iter()
        .find(|(had, _)| had == name)
        .map(|(_, path)| path)
}

/// Every name and its path, sorted, with the declared ones winning a collision.
pub fn named_dirs() -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = match NAMED.read() {
        Ok(slot) => slot
            .as_ref()
            .map(|table| {
                table
                    .iter()
                    .map(|(name, path)| (name.clone(), path.clone()))
                    .collect()
            })
            .unwrap_or_default(),
        Err(_) => Vec::new(),
    };
    let declared: Vec<String> = out.iter().map(|(name, _)| name.clone()).collect();
    out.extend(
        marks()
            .into_iter()
            .filter(|(name, _)| !declared.contains(name)),
    );
    out.sort();
    out
}

/// `@name` and `@name/tail` — the directory with anything after the first `/` kept.
///
/// `None` when the name stands for nothing, which leaves the word exactly as it was typed.
pub fn expand_at(rest: &str) -> Option<String> {
    let (name, tail) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, ""),
    };
    if name.is_empty() {
        return None;
    }
    Some(format!("{}{tail}", named_dir(name)?))
}

// ----------------------------------------------------------------- the marked half

/// `$XDG_DATA_HOME/oslo/marks`, or `~/.local/share/oslo/marks`.
///
/// A plain file rather than a row in the tracking database: a mark is three words a person chose,
/// it is worth being able to read and edit with anything, and it has no history to keep.
pub fn marks_file() -> Option<PathBuf> {
    if let Ok(slot) = MARKS_FILE.read()
        && let Some(path) = slot.as_ref()
    {
        return Some(path.clone());
    }
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .or_else(|| {
            std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share"))
        })?;
    Some(base.join("oslo/marks"))
}

static MARKS_FILE: RwLock<Option<PathBuf>> = RwLock::new(None);

/// Keep marks somewhere other than the default.
///
/// Set rather than read from `$XDG_DATA_HOME` at every call because a test must not write to the
/// person running it — `set_var` is process-wide and this crate's tests are threaded. `None` puts
/// the default back.
pub fn set_marks_file(path: Option<PathBuf>) {
    if let Ok(mut slot) = MARKS_FILE.write() {
        *slot = path;
    }
}

/// Every mark on disk, in the order the file has them.
///
/// Read on each call rather than cached: marks change from *another terminal* — that is most of
/// what they are for — and a cache would answer for the shell that happened to load it first. The
/// file is a handful of lines and the read is one `open`.
pub fn marks() -> Vec<(String, String)> {
    let Some(path) = marks_file() else {
        return Vec::new();
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|line| line.split_once('\t'))
        .filter(|(name, path)| !name.is_empty() && !path.is_empty())
        .map(|(name, path)| (name.to_string(), expand_tilde(path)))
        .collect()
}

/// The name this path is marked under, if it is marked at all.
pub fn mark_of(path: &str) -> Option<String> {
    marks()
        .into_iter()
        .find(|(_, marked)| marked == path)
        .map(|(name, _)| name)
}

/// Whether `name` is one a mark can be stored under.
///
/// The same shape a command word has, because that is where it is typed: `@work` has to lex as one
/// word, and a name with a slash in it would be read as `@name/tail` and never found.
pub fn valid_mark_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && !name.starts_with('-')
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | '+'))
}

/// Store `path` under `name`, replacing whatever that name meant.
///
/// **Re-read immediately before the write**, so a mark made in another terminal a moment ago is not
/// lost by this one — the window is a handful of microseconds rather than the length of a session.
pub fn mark(name: &str, path: &str) -> Result<(), String> {
    if !valid_mark_name(name) {
        return Err(format!(
            "{name:?} is not a mark name; letters, digits, `_`, `-`, `.` and `+`, up to 64"
        ));
    }
    if path.contains('\n') || path.contains('\t') {
        return Err(format!(
            "{path:?} cannot be marked: the file is one line each"
        ));
    }
    let mut kept: Vec<(String, String)> =
        marks().into_iter().filter(|(had, _)| had != name).collect();
    kept.push((name.to_string(), path.to_string()));
    kept.sort();
    write_marks(&kept)
}

/// Forget `name`. Answers whether there was one.
pub fn unmark(name: &str) -> Result<bool, String> {
    let before = marks();
    let kept: Vec<(String, String)> = before
        .iter()
        .filter(|(had, _)| had != name)
        .cloned()
        .collect();
    if kept.len() == before.len() {
        return Ok(false);
    }
    write_marks(&kept)?;
    Ok(true)
}

/// Replace the file, atomically.
///
/// Written beside the real file and renamed onto it, so a shell reading concurrently sees either
/// the whole old set or the whole new one — never the half a crash or a full disk left behind.
fn write_marks(marks: &[(String, String)]) -> Result<(), String> {
    let path =
        marks_file().ok_or("no $XDG_DATA_HOME and no $HOME, so there is nowhere to keep marks")?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    let temporary = path.with_extension("tmp");
    let mut file =
        std::fs::File::create(&temporary).map_err(|e| format!("{}: {e}", temporary.display()))?;
    for (name, marked) in marks {
        writeln!(file, "{name}\t{marked}").map_err(|e| format!("{}: {e}", temporary.display()))?;
    }
    file.sync_all()
        .map_err(|e| format!("{}: {e}", temporary.display()))?;
    drop(file);
    std::fs::rename(&temporary, &path).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests;

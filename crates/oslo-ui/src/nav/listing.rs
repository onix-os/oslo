//! What a directory holds, and how one line of it is drawn.
//!
//! Separated from the widget because these answer a different question: [`super::nav`] is about
//! where you are and what you pressed, and this is about what is on disk and what a row of it
//! looks like. Nothing here reads a key or draws a frame.

use super::{Icons, Row};
use crate::dropdown::{human_age, human_size};
use crate::matching::{Fuzzed, Fuzzy};
use crate::theme;
use std::path::Path;
use std::time::SystemTime;

#[derive(Debug, Clone)]
pub(super) struct Entry {
    pub(super) name: String,
    pub(super) directory: bool,
    pub(super) symlink: bool,
    pub(super) size: u64,
    pub(super) modified: SystemTime,
}

pub(super) fn read(at: &Path, hidden: bool) -> (Vec<Entry>, Option<String>) {
    let entries = match std::fs::read_dir(at) {
        Ok(entries) => entries,
        Err(error) => return (Vec::new(), Some(error.to_string())),
    };
    let mut out: Vec<Entry> = entries
        .filter_map(std::result::Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !hidden && name.starts_with('.') {
                return None;
            }
            let path = entry.path();
            let metadata = path.symlink_metadata().ok()?;
            Some(Entry {
                directory: path.is_dir(),
                symlink: metadata.file_type().is_symlink(),
                size: metadata.len(),

                modified: metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                name,
            })
        })
        .collect();
    out.sort_by(|a, b| b.directory.cmp(&a.directory).then(a.name.cmp(&b.name)));
    (out, None)
}

/// One row: a mark, the name, and — hard right — how big it is and how long ago it changed.
///
/// **The `dir`/`file` word and the `rwxrwxr-x` mode are gone.** Both were `ls -l` habits rather
/// than answers anybody wanted here: the kind is already said by the trailing `/` on a directory
/// and `@` on a link, and nine characters of mode is a question a file browser is rarely asked —
/// while together they pushed the name a third of the way across the row. What replaces them is
/// one configurable mark; see [`Icons`].
///
/// The size moved to the right for the same reason. A file browser is read down the *names*, and
/// anything to the left of them is something the eye has to skip on every row to get there. On the
/// right it sits beside the age, which is the other number, and the names all start in one place.
pub(super) fn row_of(entry: &Entry, directory_style: theme::Style, icons: &Icons) -> Row {
    let name = match (entry.directory, entry.symlink) {
        (true, true) => format!("{}@/", entry.name),
        (true, false) => format!("{}/", entry.name),
        (false, true) => format!("{}@", entry.name),
        (false, false) => entry.name.clone(),
    };
    Row {
        // The one column left of the name, where the `dir`/`file` word was, so the mark leads the
        // row and lines up down the list. An icon set to the empty string costs no column at all —
        // `meta` is sized across the listing — so turning the marks off is something a config can
        // actually say.
        meta: vec![icons.of(&entry.name, entry.directory).to_string()],
        // Padded rather than merely concatenated: `trail` is one string drawn hard right, so two
        // numbers only read as two columns if each is given a fixed width of its own. Five holds
        // every size `human_size` produces (`1023B`), four holds every age (`11mo`).
        trail: format!(
            "  {:>5}  {:>4}",
            human_size(entry.size),
            human_age(entry.modified)
        ),
        tint: entry.directory.then_some(directory_style),
        ..Row::new(name)
    }
}

pub(super) fn narrow(entries: &[Entry], query: &str, fuzzy: Fuzzy) -> Vec<usize> {
    if query.is_empty() {
        return (0..entries.len()).collect();
    }
    let pattern = Fuzzed::new(query, fuzzy);
    let mut scored: Vec<(i32, usize)> = entries
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| pattern.score(&entry.name).map(|score| (score, index)))
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    scored.into_iter().map(|(_, index)| index).collect()
}

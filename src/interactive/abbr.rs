//! Abbreviations: a short word that becomes a long one as you type it.
//!
//! ```text
//! abbr gco 'git checkout'      then typing `gco ` leaves `git checkout ` in the line
//! ```
//!
//! **Why this rather than an alias.** An alias is a language feature: it changes what a command
//! *means*, it is still an alias after you press Enter, and the history records `gco` rather than
//! what actually ran. For a shell whose promise is to be `/bin/sh` and not to change what scripts
//! see, that is the wrong shape for a typing shortcut.
//!
//! An abbreviation is purely a keyboard convenience. The real command lands in the buffer before
//! anything is parsed, so what runs, what is highlighted, what goes in the history and what a
//! script would have seen are all the same text — and you watch it happen, which is how you learn
//! the command rather than learning the shortcut.
//!
//! Two placements, which is fish's useful 20%: `command` (the default — only where a command name
//! goes) and `anywhere`. A `%` in the expansion says where the cursor should end up.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// Where an abbreviation is allowed to fire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    /// Only where a command name would go. The default, because `gco` meaning `git checkout` in
    /// the middle of a filename is a surprise nobody asked for.
    Command,
    /// Any word at all.
    Anywhere,
}

#[derive(Debug, Clone)]
pub struct Abbreviation {
    pub expansion: String,
    pub placement: Placement,
}

fn table() -> &'static Mutex<HashMap<String, Abbreviation>> {
    static ABBRS: OnceLock<Mutex<HashMap<String, Abbreviation>>> = OnceLock::new();
    ABBRS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Define one, replacing any earlier definition of the same name.
pub fn add(name: &str, expansion: &str, placement: Placement) {
    if let Ok(mut t) = table().lock() {
        t.insert(
            name.to_string(),
            Abbreviation {
                expansion: expansion.to_string(),
                placement,
            },
        );
    }
}

/// Remove one. Answers whether there was anything to remove.
pub fn remove(name: &str) -> bool {
    table()
        .lock()
        .map(|mut t| t.remove(name).is_some())
        .unwrap_or(false)
}

/// Every definition, sorted by name so `abbr` prints the same list twice running.
pub fn all() -> Vec<(String, Abbreviation)> {
    let Ok(t) = table().lock() else {
        return Vec::new();
    };
    let mut out: Vec<(String, Abbreviation)> =
        t.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Forget everything, for a config reload.
pub fn clear() {
    if let Ok(mut t) = table().lock() {
        t.clear();
    }
}

/// Expand the word ending at `cursor`, if it is an abbreviation that may fire there.
///
/// Answers the line as it should become and where the cursor should sit, or `None` to leave the
/// line exactly as it was — which is the answer for the overwhelming majority of keystrokes, so it
/// costs one hash lookup.
pub fn expand(text: &str, cursor: usize) -> Option<(String, usize)> {
    let cursor = cursor.min(text.len());
    let before = &text[..cursor];
    let start = before
        .rfind(char::is_whitespace)
        .map(|i| i + 1)
        .unwrap_or(0);
    let word = &before[start..];
    if word.is_empty() {
        return None;
    }

    let found = table().lock().ok()?.get(word).cloned()?;
    if found.placement == Placement::Command && !is_command_position(&before[..start]) {
        return None;
    }

    // `%` marks where the cursor should end up — `abbr gc 'git commit -m "%"'` leaves you inside
    // the quotes. Without one, the cursor follows the end of the expansion, which is where it
    // would have been had you typed the whole thing.
    let (expansion, offset) = match found.expansion.find('%') {
        Some(at) => (found.expansion.replacen('%', "", 1), at),
        None => (found.expansion.clone(), found.expansion.len()),
    };

    let mut out = String::with_capacity(text.len() + expansion.len());
    out.push_str(&text[..start]);
    out.push_str(&expansion);
    out.push_str(&text[cursor..]);
    Some((out, start + offset))
}

/// Whether what comes before this word means a command name goes here.
///
/// Deliberately simple: the start of the line, or just after something that begins a new command.
/// A full parse would be more correct and is not worth it — the cost of being wrong is that an
/// abbreviation does not fire, and the user types the long form, which is what they would have
/// done anyway.
fn is_command_position(before: &str) -> bool {
    let trimmed = before.trim_end();
    if trimmed.is_empty() {
        return true;
    }
    // `sudo gco` should expand too: a word that only ever precedes a command leaves the next word
    // in command position.
    let last_word = trimmed.rsplit(char::is_whitespace).next().unwrap_or("");
    if matches!(
        last_word,
        "sudo" | "doas" | "command" | "time" | "env" | "xargs"
    ) {
        return true;
    }
    trimmed.ends_with(['|', '&', ';', '(', '{'])
        || trimmed.ends_with("&&")
        || trimmed.ends_with("||")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The table is process-wide and every test here replaces it, so they cannot run beside each
    /// other — one clearing it mid-way through another looks exactly like an expansion that failed.
    static SERIAL: Mutex<()> = Mutex::new(());

    fn only(name: &str, expansion: &str, placement: Placement) {
        clear();
        add(name, expansion, placement);
    }

    /// The whole point: the short word becomes the long one, in place.
    #[test]
    fn a_word_at_the_start_of_a_line_expands() {
        let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        only("gco", "git checkout", Placement::Command);
        assert_eq!(
            expand("gco", 3),
            Some(("git checkout".to_string(), 12)),
            "cursor follows the end of the expansion"
        );
        clear();
    }

    /// A command abbreviation fires only where a command goes — including after `sudo` and after
    /// anything that starts a new command.
    #[test]
    fn placement_decides_where_it_fires() {
        let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        only("gco", "git checkout", Placement::Command);
        assert!(expand("sudo gco", 8).is_some(), "after sudo");
        assert!(expand("ls | gco", 8).is_some(), "after a pipe");
        assert!(expand("true && gco", 11).is_some(), "after &&");
        assert!(
            expand("cat gco", 7).is_none(),
            "an argument is not a command"
        );

        only("gco", "git checkout", Placement::Anywhere);
        assert!(expand("cat gco", 7).is_some(), "anywhere means anywhere");
        clear();
    }

    /// `%` says where to leave the cursor, and does not survive into the line.
    #[test]
    fn a_marker_places_the_cursor() {
        let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        only("gc", "git commit -m \"%\"", Placement::Command);
        let (text, cursor) = expand("gc", 2).expect("expands");
        assert_eq!(text, "git commit -m \"\"");
        assert_eq!(&text[cursor..], "\"", "the cursor sits inside the quotes");
        clear();
    }

    /// Text after the cursor is kept, and nothing fires for an unknown word.
    #[test]
    fn the_rest_of_the_line_is_untouched() {
        let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        only("gco", "git checkout", Placement::Command);
        assert_eq!(
            expand("gco main", 3),
            Some(("git checkout main".to_string(), 12))
        );
        assert_eq!(expand("nothing", 7), None);
        assert_eq!(expand("", 0), None);
        clear();
    }
}

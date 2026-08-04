//! Whose history this is.
//!
//! Both stores are named after a **profile** rather than after what they contain: `default.db` and
//! `default.kv`, not `history.db` and `track.kv`.
//!
//! # Why a name at all
//!
//! Because more than one thing runs commands through this shell. An agent that shells out — and
//! they all do — writes thousands of lines into the same history a person is trying to search, and
//! into the same frecency table that decides what `cd` and Tab suggest. `oslo --profile=claude` gives it
//! its own pair of stores: its history is still recorded, still searchable by pointing another
//! shell at the same profile, and no longer mixed into yours.
//!
//! It is a *profile*, not a lock. Nothing stops two shells sharing one, which is what makes
//! `oslo --profile=claude` twice in a row accumulate rather than start over.

use std::path::PathBuf;
use std::sync::RwLock;

/// The profile this process uses, once something has chosen one.
static CHOSEN: RwLock<Option<String>> = RwLock::new(None);

/// Use `name` for the rest of this process. Called once, from the invocation.
pub fn choose(name: &str) {
    let cleaned = sanitise(name);
    if cleaned.is_empty() {
        return;
    }
    if let Ok(mut slot) = CHOSEN.write() {
        *slot = Some(cleaned);
    }
}

/// The profile in force: what `--profile` asked for, or `default`.
pub fn current() -> String {
    if let Ok(slot) = CHOSEN.read()
        && let Some(name) = slot.as_ref()
    {
        return name.clone();
    }
    default_name()
}

/// What a shell uses when nothing asked for anything else.
///
/// A fixed name rather than the user's, because the profile is about *what is running the shell*,
/// not who owns it — the store already lives under that user's home. `default` also means a
/// dotfile repo can name it without knowing whose machine it is on.
fn default_name() -> String {
    "default".to_string()
}

/// A profile name reduced to something safe to put in a path.
///
/// A name reaches this from `--profile`, which can hold anything. A `/` would write the store
/// somewhere else entirely and `..` would climb out of the data directory, so the set is restricted
/// rather than escaped: everything outside it becomes `-`, which cannot traverse.
fn sanitise(name: &str) -> String {
    let kept: String = name
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.' {
                c
            } else {
                '-'
            }
        })
        .collect();
    // A name of only dots is `.` or `..`, which name directories rather than files.
    if kept.chars().all(|c| c == '.') {
        return String::new();
    }
    kept
}

/// `<data>/oslo/<profile>.<extension>`.
pub fn store_path(xdg_data: Option<&str>, home: Option<&str>, extension: &str) -> Option<PathBuf> {
    let base = match xdg_data {
        Some(dir) if !dir.trim().is_empty() => PathBuf::from(dir),
        _ => PathBuf::from(home?).join(".local/share"),
    };
    let dir = base.join("oslo");
    let path = dir.join(format!("{}.{extension}", current()));
    adopt_legacy(&dir, &path, extension);
    Some(path)
}

/// Move a store written under the old fixed name to the profile's name, once.
///
/// The stores used to be called `history.db` and `track.kv`. Renaming them without this would
/// leave a person's whole history sitting in a file nothing looks at any more — the shell would
/// come up empty and nothing would say why.
///
/// Only for the **default** profile, and only when the new name does not exist: `--profile=claude`
/// must never adopt your history, and a profile that already has a store is not touched. A rename
/// rather than a copy, so it happens once and the old name does not linger to be adopted twice.
fn adopt_legacy(dir: &std::path::Path, wanted: &std::path::Path, extension: &str) {
    if wanted.exists() || current() != default_name() {
        return;
    }
    let legacy = match extension {
        "db" => dir.join("history.db"),
        "kv" => dir.join("track.kv"),
        _ => return,
    };
    if legacy.is_file() {
        // A failure here is not worth a diagnostic on every prompt: the shell simply starts with
        // an empty store, which is what would have happened anyway.
        let _ = std::fs::rename(&legacy, wanted);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A name that could climb out of the data directory cannot.
    #[test]
    fn a_name_cannot_escape_the_directory() {
        // Dots survive — `v1.2` is a reasonable profile name — but the separator does not, so
        // there is nothing left to traverse with.
        assert_eq!(sanitise("../../etc/passwd"), "..-..-etc-passwd");
        assert_eq!(sanitise("a/b"), "a-b");
        assert_eq!(sanitise(".."), "", "a name of only dots is not a name");
        assert_eq!(sanitise("."), "");
        for hostile in ["../../etc/passwd", "a/b", "~/x", "a\\b"] {
            let cleaned = sanitise(hostile);
            assert!(!cleaned.contains('/'), "{cleaned}");
            assert!(!cleaned.contains('\\'), "{cleaned}");
        }
    }

    /// The ordinary names survive untouched — this must not mangle what people actually type.
    #[test]
    fn ordinary_names_are_left_alone() {
        for name in ["bresilla", "claude", "agent-1", "test_run", "v1.2"] {
            assert_eq!(sanitise(name), name);
        }
    }

    /// Nothing chosen means `default`, which is what an ordinary shell writes to.
    #[test]
    fn the_default_profile_is_called_default() {
        assert_eq!(default_name(), "default");
    }

    /// The path is the profile with the store's extension, under the data directory.
    #[test]
    fn the_path_is_the_profile_and_the_extension() {
        choose("claude");
        assert_eq!(
            store_path(Some("/x/data"), None, "db"),
            Some(PathBuf::from("/x/data/oslo/claude.db"))
        );
        assert_eq!(
            store_path(None, Some("/home/u"), "kv"),
            Some(PathBuf::from("/home/u/.local/share/oslo/claude.kv"))
        );
        // And the two stores of one profile sit beside each other, differing only in extension.
        let db = store_path(Some("/x"), None, "db").expect("a path");
        let kv = store_path(Some("/x"), None, "kv").expect("a path");
        assert_eq!(db.parent(), kv.parent());
        assert_eq!(db.file_stem(), kv.file_stem());
    }

    /// With nowhere to put it, there is no path — not a path in the wrong place.
    #[test]
    fn no_home_is_no_path() {
        assert_eq!(store_path(None, None, "db"), None);
    }
}

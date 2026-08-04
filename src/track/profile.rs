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
//!
//! Either `--profile=NAME` or `$OSLO_PROFILE` names it — the flag for one invocation, the variable
//! for a whole session, and the flag wins when both are given.

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

/// The environment variable that names a profile without a flag.
pub const ENV: &str = "OSLO_PROFILE";

/// The profile in force.
///
/// `--profile` wins, then `$OSLO_PROFILE`, then `default`. That order is the usual one and it is
/// the useful one here: the variable is how you put a whole *session* on a profile — export it
/// once and every `oslo` a tool spawns inherits it — and the flag is how you override that for one
/// invocation without disturbing the session.
///
/// Read rather than cached, so exporting it mid-session takes effect on the next shell without
/// anything having to be told.
pub fn current() -> String {
    if let Ok(slot) = CHOSEN.read()
        && let Some(name) = slot.as_ref()
    {
        return name.clone();
    }
    if let Ok(name) = std::env::var(ENV) {
        let cleaned = sanitise(&name);
        if !cleaned.is_empty() {
            return cleaned;
        }
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
    Some(base.join("oslo").join(format!("{}.{extension}", current())))
}

/// The directory both stores live in.
pub fn store_dir(xdg_data: Option<&str>, home: Option<&str>) -> Option<PathBuf> {
    let base = match xdg_data {
        Some(dir) if !dir.trim().is_empty() => PathBuf::from(dir),
        _ => PathBuf::from(home?).join(".local/share"),
    };
    Some(base.join("oslo"))
}

/// Every profile that has a tracking store, sorted, with the current one always present.
///
/// Found by listing rather than recorded anywhere: a profile *is* a pair of files, so the
/// directory is the only thing that could be authoritative. The current profile is included even
/// with nothing written yet, or a brand-new shell would have nothing to switch away from.
pub fn available() -> Vec<String> {
    let mut found: Vec<String> = std::env::var("XDG_DATA_HOME")
        .ok()
        .or_else(|| std::env::var("HOME").ok())
        .and_then(|_| {
            store_dir(
                std::env::var("XDG_DATA_HOME").ok().as_deref(),
                std::env::var("HOME").ok().as_deref(),
            )
        })
        .and_then(|dir| std::fs::read_dir(dir).ok())
        .map(|entries| {
            entries
                .flatten()
                .filter_map(|entry| {
                    let path = entry.path();
                    // The aggregate is the one that must exist for a profile to be worth showing:
                    // an event log with no store has nothing the finder can rank.
                    (path.extension()? == "kv")
                        .then(|| path.file_stem()?.to_str().map(str::to_string))
                        .flatten()
                })
                .collect()
        })
        .unwrap_or_default();
    let now = current();
    if !found.contains(&now) {
        found.push(now);
    }
    found.sort();
    found.dedup();
    found
}

/// The profile after `name` in the list, wrapping. `None` when there is only one.
pub fn after(name: &str) -> Option<String> {
    let all = available();
    if all.len() < 2 {
        return None;
    }
    let at = all.iter().position(|found| found == name).unwrap_or(0);
    all.get((at + 1) % all.len()).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The profile is process-wide state, so the tests that set it cannot run beside each other.
    static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

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

    /// `$OSLO_PROFILE` names a profile for a whole session; `--profile` overrides it for one
    /// invocation. Both are cleaned the same way, so neither can escape the directory.
    #[test]
    fn the_environment_names_a_profile_and_the_flag_beats_it() {
        // SAFETY: as elsewhere in this crate's tests — see `env::scope::environ`. The lock below
        // is what keeps this from racing the other tests that read the profile.
        let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        *CHOSEN.write().expect("chosen") = None;

        unsafe { std::env::set_var(ENV, "from-the-env") };
        assert_eq!(current(), "from-the-env");

        // A hostile value is cleaned here too, not only on the flag.
        unsafe { std::env::set_var(ENV, "../escape") };
        assert_eq!(current(), "..-escape");

        // Empty means unset: fall through rather than writing to a file called nothing.
        unsafe { std::env::set_var(ENV, "   ") };
        assert_eq!(current(), "default");

        unsafe { std::env::set_var(ENV, "from-the-env") };
        choose("from-the-flag");
        assert_eq!(current(), "from-the-flag", "the flag wins");

        *CHOSEN.write().expect("chosen") = None;
        unsafe { std::env::remove_var(ENV) };
    }

    /// The path is the profile with the store's extension, under the data directory.
    #[test]
    fn the_path_is_the_profile_and_the_extension() {
        let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
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

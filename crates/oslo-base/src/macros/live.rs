//! What a running shell should have in hand right now, and how it finds out that it changed.
//!
//! # Two sources, and one of them is a file only a shell can write
//!
//! An alias reaches a shell from the database — [`mod@super::snapshot`] — or from the configuration:
//! `alias` in a bash file, `oslo.alias` in Lua. The second cannot be enumerated by anything except a
//! shell that has run its config, so a shell writes it down as it starts, into `elsewhere.snapshot`
//! beside the other. That file is what lets `oslo macros` show you a configured alias next to a
//! stored one, and it is what lets a *removal* put the configured one back rather than leaving a
//! hole.
//!
//! # Noticing, for the price of a stat
//!
//! Universal variables already solve this exact problem — write it from anywhere, see it everywhere
//! — and the mechanism is one `stat` per prompt on a file that is read only when it has moved. This
//! is the same mechanism, deliberately: turning a macro off in one terminal takes effect in the one
//! beside it before its next prompt, and a shell where nothing changed pays two `stat`s and no
//! parse.
//!
//! ```text
//! per prompt:  stat macros.snapshot ─┐
//!              stat <session>.off  ──┴─ moved? ─► want():  elsewhere, then stored on top,
//!                                                          minus anything off for this session
//! ```
//!
//! # Why the answer is rebuilt rather than patched
//!
//! Because the interesting case is *removal*, and a patch cannot see one. When `gs` leaves the
//! database, the right answer is whatever the configuration said about `gs` — which may be nothing,
//! or may be a different alias entirely. Composing the whole set from both sources and diffing it
//! against what this shell was last given answers that without anyone having to remember what
//! overwrote what.
//!
//! **An alias you typed is not ours to touch.** [`Applied`] remembers only the names this put there,
//! so `alias gs='...'` at the prompt survives every rebuild — it is not in the set, so it is never
//! in the difference.

use super::{Entry, Kind, snapshot};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::SystemTime;

/// `<data>/oslo/macros/elsewhere.snapshot` — what the configuration defined, written by a shell.
pub fn elsewhere() -> Option<PathBuf> {
    Some(super::directory()?.join("elsewhere.snapshot"))
}

/// Record what the configuration defined, before anything from the database is applied over it.
pub fn publish_elsewhere(entries: &[Entry]) -> Result<(), String> {
    let path = elsewhere().ok_or_else(|| "nowhere to write what the config defined".to_string())?;
    snapshot::write_to(&path, entries)
}

/// What the configuration defined, as last written by a starting shell.
pub fn from_elsewhere() -> Vec<Entry> {
    match elsewhere() {
        Some(path) => snapshot::read_from(&path),
        None => Vec::new(),
    }
}

/// Everything a shell should have in hand: the configuration's, then the database's on top.
///
/// **The database wins**, because it is the one you can change without editing a file: that is what
/// makes `oslo macros add gco …` taking effect the expected outcome rather than the surprising one.
/// `off` names are dropped last, so turning something off uncovers the configured one underneath —
/// which is the same rule, applied to a removal.
pub fn want() -> Vec<Entry> {
    let mut by_key: BTreeMap<(Kind, String), Entry> = BTreeMap::new();
    for entry in from_elsewhere() {
        by_key.insert((entry.kind, entry.name.clone()), entry);
    }
    for entry in snapshot::read() {
        by_key.insert((entry.kind, entry.name.clone()), entry);
    }
    let off = session::off();
    by_key
        .into_values()
        .filter(|entry| !off.contains(&entry.name))
        .collect()
}

/// The names this put into a shell, so the next rebuild knows what is its to take away.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Applied {
    pub aliases: Vec<String>,
    pub abbrevs: Vec<String>,
    pub vars: Vec<String>,
}

impl Applied {
    pub fn of(entries: &[Entry]) -> Applied {
        let names = |kind: Kind| -> Vec<String> {
            entries
                .iter()
                .filter(|e| e.kind == kind)
                .map(|e| e.name.clone())
                .collect()
        };
        Applied {
            aliases: names(Kind::Alias),
            abbrevs: names(Kind::Abbrev),
            vars: names(Kind::Var),
        }
    }

    /// The names that were ours and are not wanted any more.
    ///
    /// **A variable that has already been read is not taken back.** By then it is an ordinary
    /// exported variable and the recipe is gone; removing the value would be reaching into a shell
    /// to unset something a command may be holding. What is dropped is the *recipe*, so the name
    /// stops resolving to it in every shell that has not asked yet.
    pub fn gone(&self, next: &Applied) -> Applied {
        let dropped = |had: &[String], now: &[String]| -> Vec<String> {
            had.iter()
                .filter(|name| !now.contains(name))
                .cloned()
                .collect()
        };
        Applied {
            aliases: dropped(&self.aliases, &next.aliases),
            abbrevs: dropped(&self.abbrevs, &next.abbrevs),
            vars: dropped(&self.vars, &next.vars),
        }
    }
}

/// When the two files last moved. Compared, never read for their own sake.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Stamps {
    stored: Option<SystemTime>,
    off: Option<SystemTime>,
}

impl Stamps {
    /// A `stat` on each, and nothing else. This is what runs on every prompt.
    pub fn now() -> Stamps {
        Stamps {
            stored: moved(super::snapshot()),
            off: moved(session::path()),
        }
    }
}

fn moved(path: Option<PathBuf>) -> Option<SystemTime> {
    std::fs::metadata(path?).ok()?.modified().ok()
}

/// Turning one off for **this shell only**.
///
/// A file per session, holding one name per line. It is this and not a message to the shell because
/// `oslo macros` is a *separate process* — a child of the shell whose macros it is managing — and
/// the only thing the two reliably share is the session id and the filesystem.
pub mod session {
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    /// `<data>/oslo/macros/session/<id>.off`.
    pub fn path() -> Option<PathBuf> {
        let id = crate::track::session::id();
        Some(
            super::super::directory()?
                .join("session")
                .join(format!("{id}.off")),
        )
    }

    /// The names this shell has turned off, in the order they will be looked up.
    pub fn off() -> BTreeSet<String> {
        let Some(path) = path() else {
            return BTreeSet::new();
        };
        let Ok(text) = std::fs::read_to_string(path) else {
            return BTreeSet::new();
        };
        text.lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_string)
            .collect()
    }

    /// Turn `name` off, or back on, for the session `id`.
    ///
    /// The session is named rather than taken from this process, because the process doing the
    /// turning off is the manager and the session it means is its parent's.
    pub fn set(id: &str, name: &str, off: bool) -> Result<(), String> {
        let Some(directory) = super::super::directory().map(|dir| dir.join("session")) else {
            return Err("nowhere to keep the session list".to_string());
        };
        std::fs::create_dir_all(&directory).map_err(|e| format!("{}: {e}", directory.display()))?;
        let path = directory.join(format!("{id}.off"));
        let mut names: BTreeSet<String> = std::fs::read_to_string(&path)
            .map(|text| text.lines().map(str::to_string).collect())
            .unwrap_or_default();
        if off {
            names.insert(name.to_string());
        } else {
            names.remove(name);
        }
        let mut text = names.into_iter().collect::<Vec<_>>().join("\n");
        if !text.is_empty() {
            text.push('\n');
        }
        // **Written even when it is empty**, because the shell watches the file's mtime: turning
        // the last one back on has to move the file, or the shell never finds out.
        std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Forget the lists of sessions that ended long ago.
    ///
    /// A session file outlives its shell — there is no reliable moment to delete one, since a shell
    /// can be killed — so they are swept by age instead, from a starting shell. A week is well past
    /// any session still running and well short of anything worth keeping: what it holds is "off
    /// until this terminal closes".
    pub fn sweep() {
        const WEEK: u64 = 7 * 24 * 60 * 60;
        let Some(directory) = super::super::directory().map(|dir| dir.join("session")) else {
            return;
        };
        let Ok(entries) = std::fs::read_dir(&directory) else {
            return;
        };
        let mine = path();
        for entry in entries.flatten() {
            let old = entry
                .metadata()
                .and_then(|meta| meta.modified())
                .map(|when| when.elapsed().map(|since| since.as_secs()).unwrap_or(0) > WEEK)
                .unwrap_or(false);
            // Never this shell's own, whatever its timestamp says: a clock that moved backwards is
            // not a reason to turn somebody's macros back on underneath them.
            if old && mine.as_deref() != Some(entry.path().as_path()) {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }

    /// Whether `name` is off in the session `id`.
    pub fn is_off(id: &str, name: &str) -> bool {
        let Some(directory) = super::super::directory().map(|dir| dir.join("session")) else {
            return false;
        };
        std::fs::read_to_string(directory.join(format!("{id}.off")))
            .map(|text| text.lines().any(|line| line.trim() == name))
            .unwrap_or(false)
    }
}

#[cfg(test)]
#[path = "live/tests.rs"]
mod tests;

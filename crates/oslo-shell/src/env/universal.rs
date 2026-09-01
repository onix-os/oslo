//! Universal variables: set once, seen by every session, still there after a reboot.
//!
//! ```sh
//! set -U theme dark      # here, and in every other oslo window
//! set -U                 # what is in the store
//! set -U -e theme        # gone, everywhere
//! ```
//!
//! # What this is, next to the two things that look like it
//!
//! oslo already has two mechanisms that cross a boundary and neither is this one. The control
//! socket *asks another shell* a question — a request and an answer, both sides live. `profile
//! sync` moves data *between machines*, on demand. A universal variable is neither: it is one
//! value, on one machine, that every session sees without any of them asking each other anything.
//!
//! # No daemon, and nothing new linked
//!
//! One file per user, replaced atomically, and re-read when it has changed. There is no process in
//! the middle, so there is nothing to start, nothing to fail to start, and no state that outlives
//! the sessions using it.
//!
//! ```text
//!   $XDG_STATE_HOME/oslo/universal
//!        │
//!        ├─ read    two stats decide whether to parse at all
//!        ├─ write   a temporary in the same directory, then rename(2)
//!        └─ sync    the difference, applied to a session's own variables
//! ```
//!
//! # The failure modes, written down first
//!
//! This is the one part of the feature where the obvious implementation is subtly wrong, and wrong
//! silently. So:
//!
//! * **Two shells writing at once.** `rename(2)` makes each write whole — nobody ever reads half a
//!   file — but the second writer's copy wins and the first one's change is gone. That is the
//!   trade fish makes too. Merging per key would be better and is a different feature: it needs a
//!   lock or a log, and both are things that can be left behind by a shell that was killed.
//! * **A session that has not looked recently.** Every read revalidates against the file's
//!   identity and size before answering, so "stale" lasts until the next access rather than until
//!   something notifies. A shell inside a long foreground job is exactly that case.
//! * **A file that is corrupt or truncated.** The parse either succeeds whole or is discarded
//!   whole, and a discarded parse **leaves the session's variables exactly as they were**. A store
//!   that cannot be read must never look like a store that was emptied — that is the difference
//!   between a bad afternoon and a lost `$PATH`.
//!
//! # Why a stat and not an inotify watch
//!
//! The plan called for `inotify`, which `nix` already provides. What it buys over revalidating on
//! access is *immediacy without an access*: a status line that redraws the instant another window
//! changed something, rather than at the next prompt. What it costs is a descriptor in the event
//! loop, a queue that a shell inside a long job does not drain, and a second path to the same
//! answer — and the stale-queue case still needs the stat, because a watch nobody is reading is
//! not a watch. Two stats per prompt is what that immediacy would have saved, so the stats are
//! what this does. If something ever needs to know between one prompt and the next, the watch goes
//! in beside this rather than instead of it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// What a session last saw, and how it knows whether that is still true.
///
/// Thread-local because only the shell's own thread reads or writes variables, and process-wide
/// state shared with nobody is a lock nobody needs.
#[derive(Default)]
struct Seen {
    values: BTreeMap<String, String>,
    stamp: Option<Stamp>,
    /// Whether anything has been read at all. A first sync hands the session everything, where a
    /// later one hands it only what changed.
    read: bool,
}

/// A file's identity and size, which is what "has this changed?" means here.
///
/// Modification time alone is not enough: a store written twice inside one timestamp tick is a
/// real thing on a filesystem with second granularity, and the length catches the ordinary case of
/// it. The inode catches the replacement, which is what every write here actually is.
#[derive(PartialEq, Eq, Clone, Copy)]
struct Stamp {
    inode: u64,
    len: u64,
    modified: std::time::SystemTime,
}

thread_local! {
    static SEEN: std::cell::RefCell<Seen> = std::cell::RefCell::new(Seen::default());
}

/// The file every session shares.
///
/// `$XDG_STATE_HOME`, because that is what state is: not configuration, which a person edits and
/// keeps in version control, and not data, which `make configs` mirrors with `rsync --delete`.
pub fn path() -> Option<PathBuf> {
    if let Ok(named) = std::env::var("OSLO_UNIVERSAL")
        && !named.is_empty()
    {
        return Some(PathBuf::from(named));
    }
    let state = match std::env::var("XDG_STATE_HOME") {
        Ok(path) if path.starts_with('/') => PathBuf::from(path),
        _ => PathBuf::from(std::env::var("HOME").ok()?).join(".local/state"),
    };
    Some(state.join("oslo/universal"))
}

/// Everything in the store, as it stands right now.
pub fn all() -> BTreeMap<String, String> {
    reload();
    SEEN.with(|seen| seen.borrow().values.clone())
}

/// One variable's value, if the store has it.
pub fn get(name: &str) -> Option<String> {
    reload();
    SEEN.with(|seen| seen.borrow().values.get(name).cloned())
}

/// Set one, everywhere.
///
/// Read, change, write — and the read is deliberately of the *file* rather than of the snapshot,
/// so a value another session added a moment ago is carried forward rather than dropped.
pub fn set(name: &str, value: &str) -> Result<(), String> {
    change(|values| {
        values.insert(name.to_string(), value.to_string());
    })
}

/// Erase one, everywhere. `false` if there was nothing of that name.
pub fn erase(name: &str) -> Result<bool, String> {
    let mut had = false;
    change(|values| {
        had = values.remove(name).is_some();
    })?;
    Ok(had)
}

/// Read the file, apply `edit`, write it back atomically.
fn change(edit: impl FnOnce(&mut BTreeMap<String, String>)) -> Result<(), String> {
    let path = path().ok_or_else(|| "there is nowhere to keep it; set $HOME".to_string())?;
    let mut values = match read(&path) {
        Some((values, _)) => values,
        // Nothing readable there yet, which is the ordinary state of a machine that has never set
        // one. A file that exists and will not parse is the same answer, and the reason is in the
        // module note: this writes a whole store, so a broken one is replaced rather than merged.
        None => BTreeMap::new(),
    };
    edit(&mut values);
    write(&path, &values)?;
    // **The writer's snapshot moves with the file**, so its own next [`sync_into`] has nothing to
    // say about a change it made itself. The session that typed `set -U` was told by the builtin,
    // which set the variable and announced it as local; hearing about it again a prompt later, as
    // if another window had done it, is a lie about where it came from.
    SEEN.with(|seen| {
        let mut seen = seen.borrow_mut();
        seen.values = values;
        seen.stamp = stamp(&path);
        seen.read = true;
    });
    Ok(())
}

/// Bring `env`'s variables up to date with the store, and say what changed.
///
/// **Only what changed.** A universal variable becomes an ordinary shell variable in each session,
/// so overwriting all of them on every prompt would undo `x=2` typed a second ago — the store is
/// where the value lives, not what the session is allowed to be doing with it.
pub fn sync_into(env: &mut crate::env::scope::Environment) -> Vec<Change> {
    let before = SEEN.with(|seen| seen.borrow().values.clone());
    let first = SEEN.with(|seen| !seen.borrow().read);
    if !reload() && !first {
        return Vec::new();
    }
    let after = SEEN.with(|seen| seen.borrow().values.clone());

    let mut changes = Vec::new();
    for (name, value) in &after {
        if before.get(name) != Some(value) {
            env.set_var(name, value, false);
            announce(name, crate::env::announce::Change::Set { exported: false });
            changes.push(Change::Set(name.clone(), value.clone()));
        }
    }
    for name in before.keys() {
        if !after.contains_key(name) {
            env.unset_var(name);
            announce(name, crate::env::announce::Change::Erased);
            changes.push(Change::Erased(name.clone()));
        }
    }
    changes
}

/// Tell `on-variable-change` that another shell did this.
///
/// **`Remote`, and that is the field this hook was given a `source` for.** A status line that
/// redraws when the value it shows changed in another window, and does not redraw for the `x=1` you
/// just typed, cannot tell those apart any other way.
fn announce(name: &str, change: crate::env::announce::Change) {
    crate::env::announce::announce(
        name,
        change,
        crate::env::announce::Scope::Stored,
        crate::env::announce::Source::Remote,
    );
}

/// What one sync did to one variable.
#[derive(Debug, PartialEq, Eq)]
pub enum Change {
    Set(String, String),
    Erased(String),
}

impl Change {
    pub fn name(&self) -> &str {
        match self {
            Change::Set(name, _) | Change::Erased(name) => name,
        }
    }
}

/// Re-read if the file is not the one already read. `true` if the snapshot moved.
///
/// **The stat is the whole point.** The common case is that nobody changed anything, and it costs
/// one `stat` and no parse.
fn reload() -> bool {
    let Some(path) = path() else {
        return false;
    };
    let now = stamp(&path);
    let unchanged = SEEN.with(|seen| {
        let seen = seen.borrow();
        seen.read && seen.stamp == now
    });
    if unchanged {
        return false;
    }
    match read(&path) {
        Some((values, stamp)) => SEEN.with(|seen| {
            let mut seen = seen.borrow_mut();
            let moved = !seen.read || seen.values != values;
            seen.values = values;
            seen.stamp = stamp;
            seen.read = true;
            moved
        }),
        // **Unreadable is not empty.** A store that will not parse leaves the session with what it
        // already had; only a file that is genuinely gone empties the snapshot.
        None => SEEN.with(|seen| {
            let mut seen = seen.borrow_mut();
            if path.exists() {
                return false;
            }
            let moved = !seen.read || !seen.values.is_empty();
            seen.values.clear();
            seen.stamp = None;
            seen.read = true;
            moved
        }),
    }
}

fn stamp(path: &Path) -> Option<Stamp> {
    use std::os::unix::fs::MetadataExt;
    let meta = std::fs::metadata(path).ok()?;
    Some(Stamp {
        inode: meta.ino(),
        len: meta.len(),
        modified: meta.modified().ok()?,
    })
}

/// The file's first line, which says what the rest of it is.
const HEADING: &str = "# oslo universal variables, one per line: NAME<TAB>VALUE";

/// The store, or `None` if there is nothing readable there.
fn read(path: &Path) -> Option<(BTreeMap<String, String>, Option<Stamp>)> {
    // Stamped **before** the read, so a write that lands between the two makes the snapshot look
    // stale rather than making a stale snapshot look current.
    let taken = stamp(path);
    let text = std::fs::read_to_string(path).ok()?;
    let values = parse(&text)?;
    Some((values, taken))
}

/// Read the file's text. `None` if any line of it is not what this writes.
///
/// **Whole or nothing.** A half-written store cannot happen through `rename(2)`, but a file edited
/// by hand or left by an older version can be anything, and applying the half of it that parsed
/// would erase whatever the rest of it held.
fn parse(text: &str) -> Option<BTreeMap<String, String>> {
    let mut values = BTreeMap::new();
    for line in text.lines() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (name, value) = line.split_once('\t')?;
        if !crate::env::scope::is_valid_identifier(name) {
            return None;
        }
        values.insert(name.to_string(), unescape(value)?);
    }
    Some(values)
}

/// Write the whole store, atomically.
///
/// A temporary **in the same directory**, because `rename(2)` is only atomic within a filesystem
/// and `$TMPDIR` is routinely a different one. The name carries the pid, so two shells writing at
/// the same moment write to two different temporaries and only the rename races — which is the
/// race this can actually survive.
fn write(path: &Path, values: &BTreeMap<String, String>) -> Result<(), String> {
    let directory = path.parent().ok_or_else(|| "no directory".to_string())?;
    std::fs::create_dir_all(directory).map_err(|e| format!("{}: {e}", directory.display()))?;

    let mut text = String::from(HEADING);
    text.push('\n');
    for (name, value) in values {
        text.push_str(name);
        text.push('\t');
        text.push_str(&escape(value));
        text.push('\n');
    }

    let temporary = directory.join(format!(".universal.{}", std::process::id()));
    std::fs::write(&temporary, &text).map_err(|e| format!("{}: {e}", temporary.display()))?;
    match std::fs::rename(&temporary, path) {
        Ok(()) => Ok(()),
        Err(problem) => {
            let _ = std::fs::remove_file(&temporary);
            Err(format!("{}: {problem}", path.display()))
        }
    }
}

/// A value on one line. A newline or a tab in it would be a second entry, so both are spelled out.
fn escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            other => out.push(other),
        }
    }
    out
}

/// The inverse. `None` for an escape this does not write, which makes the line unreadable and the
/// whole file with it — see [`parse`].
fn unescape(text: &str) -> Option<String> {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next()? {
            '\\' => out.push('\\'),
            'n' => out.push('\n'),
            't' => out.push('\t'),
            'r' => out.push('\r'),
            _ => return None,
        }
    }
    Some(out)
}

/// Forget what this session has read. For the tests, which share a thread.
#[cfg(test)]
fn forget() {
    SEEN.with(|seen| *seen.borrow_mut() = Seen::default());
}

#[cfg(test)]
#[path = "universal/tests.rs"]
mod tests;

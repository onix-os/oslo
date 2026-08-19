//! Variables that outlive the shell and reach the ones already running.
//!
//! fish's `set -U`. A variable set here is written to a file, so it is there in the next session,
//! and every other oslo re-reads that file before drawing its next prompt — so setting a theme in
//! one terminal changes the terminal beside it without either being restarted.
//!
//! # Why this is not just a file the config sources
//!
//! Because the two halves people actually want are *write it from anywhere* and *see it
//! everywhere*, and a sourced file gives neither. Editing a config from a running shell means
//! reloading every other shell by hand, which nobody does, so in practice the setting applies to
//! terminals opened after it — the least useful moment.
//!
//! # The file
//!
//! `$XDG_DATA_HOME/oslo/universal` (or `~/.local/share/…`), one variable per line:
//!
//! ```text
//! x THEME=dark
//! - EDITOR=hx
//! ```
//!
//! The flag column is `x` for a variable that is also exported to children and `-` for one that is
//! not. Values are escaped so a newline cannot end a line early — which is the whole reason this is
//! a format rather than `NAME=value` and a hope.
//!
//! It is **not** shell syntax and is never sourced. A file that is executed is a file that can be
//! made to execute something, and this one is written by every shell you have open.
//!
//! # Two shells writing at once
//!
//! Every change is a read-modify-write under an exclusive `flock`, so a second shell setting a
//! different variable at the same moment cannot lose the first one's. Rewriting the whole file
//! without the lock would make the last writer's copy — missing whatever it never read — the
//! surviving one.
//!
//! The lock is a file of its own beside the store, because the store is **replaced** rather than
//! edited: the new contents are written next to it and renamed over it. A reader takes no lock at
//! all — one `stat` and usually no read — so it has to be impossible for one to see a half-written
//! file, and the rename is what makes it so.

pub mod watch;

use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;

/// A universal variable: its value, and whether children inherit it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Universal {
    pub value: String,
    pub exported: bool,
}

/// A directory the tests point the store at, instead of the real one.
///
/// A seam rather than `XDG_DATA_HOME`, because setting an environment variable from a test writes
/// the **real** process environment, which libtest's sibling threads share — the same hazard that
/// made `ui spin` fail intermittently. Two tests here would simply overwrite each other's store.
#[cfg(test)]
static ROOT: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);

#[cfg(test)]
fn root_override() -> Option<PathBuf> {
    ROOT.lock().ok()?.clone()
}

/// The store is one file and one process-wide record, so tests that touch it cannot run beside
/// each other. This hands out an isolated store and the lock that keeps it isolated.
#[cfg(test)]
pub fn isolate_for_tests() -> (std::sync::MutexGuard<'static, ()>, tempfile::TempDir) {
    static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let guard = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().expect("tempdir");
    *ROOT.lock().expect("root") = Some(dir.path().to_path_buf());
    with_applied(|applied| applied.clear());
    // The servicer's memory is about the *old* store, and a fresh one makes it a lie.
    if let Ok(mut serviced) = SERVICED.lock() {
        *serviced = None;
    }
    (guard, dir)
}

#[cfg(not(test))]
fn root_override() -> Option<PathBuf> {
    None
}

/// Where the file lives, or `None` when there is no home to put it in.
pub fn path() -> Option<PathBuf> {
    if let Some(root) = root_override() {
        return Some(root.join("oslo/universal"));
    }
    let base = std::env::var("XDG_DATA_HOME")
        .ok()
        .filter(|x| !x.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            let home = std::env::var("HOME").ok().filter(|h| !h.is_empty())?;
            Some(PathBuf::from(home).join(".local/share"))
        })?;
    Some(base.join("oslo/universal"))
}

mod format;

use format::{parse, render};

use crate::env::announce::{Change, Scope, Source, announce};

/// What this shell has applied from the file: name to the value it wrote.
///
/// Process state rather than a parameter, because there are **two** writers. The loop applies what
/// it reads; the `universal` builtin applies what you just typed. If the builtin did not record its
/// own write, the next reload would see a name it had never applied, conclude the user had assigned
/// it by hand, and leave it alone for the rest of the session — so erasing it from another terminal
/// would never reach the terminal that created it.
static APPLIED: std::sync::Mutex<Option<std::collections::HashMap<String, String>>> =
    std::sync::Mutex::new(None);

fn with_applied<T>(
    work: impl FnOnce(&mut std::collections::HashMap<String, String>) -> T,
) -> Option<T> {
    let mut slot = APPLIED.lock().ok()?;
    Some(work(slot.get_or_insert_with(Default::default)))
}

/// Record that this shell has put `value` in `name` on the file's behalf.
pub fn note_applied(name: &str, value: &str) {
    with_applied(|applied| applied.insert(name.to_string(), value.to_string()));
}

/// Forget that we manage `name`, for the shell that just erased it.
pub fn forget_applied(name: &str) {
    with_applied(|applied| applied.remove(name));
}

/// The last stamp the *background* servicer acted on, and whether it has ever looked.
///
/// Its own, separate from the REPL's: the two ask the same question at different moments, and both
/// answers are only ever "re-read or do not". Sharing one would mean the servicer's read could stop
/// the loop's, and re-reading is idempotent anyway.
///
/// **Two layers of `Option`, and the outer one is load-bearing.** The inner is the stamp, which is
/// `None` when there is no store file — the ordinary state of a machine that has never set a
/// universal variable. The outer says whether this has been asked before. Collapsing them made
/// "no file" indistinguishable from "never looked", so every call reported a change, and every
/// idle wake told the prompt to rebuild itself. With an external prompt that spawns, the rebuild
/// spawned a child, the child's exit woke the editor, and the shell rendered its prompt three
/// hundred times a second for as long as it sat there.
static SERVICED: std::sync::Mutex<Option<Option<Stamp>>> = std::sync::Mutex::new(None);

/// Re-read and apply, but only if the file has actually moved since this last looked.
///
/// **For the idle wake.** `inotify` says the directory changed, which is not the same as saying
/// *this* file did — a lock file, a scratch file mid-rename and the store itself all live there, and
/// every one of them is a wake. One `stat` decides.
pub fn apply_if_changed(env: &mut crate::env::Environment) -> bool {
    let now = changed_at();
    let mut serviced = match SERVICED.lock() {
        Ok(serviced) => serviced,
        Err(poisoned) => poisoned.into_inner(),
    };
    if *serviced == Some(now) {
        return false;
    }
    *serviced = Some(now);
    drop(serviced);
    apply(env);
    true
}

/// Apply every universal variable to `env`, and take away the ones that have been erased.
///
/// **An assignment in this shell wins.** A universal variable is a default that follows you
/// around, not a lock: `PAGER=cat some-command` has to mean `cat` for that command even if a
/// universal `PAGER` exists, and a shell that reversed that would be one nobody could override.
///
/// Telling the two apart is what the `APPLIED` record above is for. A name whose current value is
/// still exactly what this shell last wrote is ours to change; a name the user has since assigned
/// to is not, and is left alone in both directions — not updated, and not removed when the file
/// drops it.
pub fn apply(env: &mut crate::env::Environment) {
    let file = load();
    let mut ours: Vec<String> = Vec::new();
    let mut gone: Vec<(String, String)> = Vec::new();

    with_applied(|applied| {
        for name in file.keys() {
            let last = applied.get(name);
            match env.get_var(name) {
                // Set here by the user, not by us: theirs.
                Some(current) if last.is_none_or(|last| last != current) => continue,
                _ => {}
            }
            ours.push(name.clone());
        }
        // Erased elsewhere. Without this, `universal -e X` in one terminal would leave `X` set in
        // every other one until it was restarted — so the erase would look like it had worked and
        // then be contradicted by the next shell you looked at.
        applied.retain(|name, last| {
            if file.contains_key(name) {
                return true;
            }
            gone.push((name.clone(), last.clone()));
            false
        });
    });

    for name in ours {
        let var = &file[&name];
        // **Only when the value actually moved.** This runs before every prompt and every command,
        // so announcing what it read rather than what changed would fire the hook continuously at
        // an idle prompt — the store says the same thing each time.
        let arrived = env.get_var(&name) != Some(var.value.as_str());
        env.set_var(&name, &var.value, var.exported);
        note_applied(&name, &var.value);
        if arrived {
            announce(
                &name,
                Change::Set {
                    exported: var.exported,
                },
                Scope::Universal,
                Source::Remote,
            );
        }
    }
    for (name, last) in gone {
        if env.get_var(&name).is_some_and(|current| current == last) {
            env.unset_var(&name);
            announce(&name, Change::Erased, Scope::Universal, Source::Remote);
        }
    }
}

/// Re-read and apply if the file has moved since `stamp`, answering the new stamp.
///
/// Called at **two** moments, and both are needed. Before a prompt, so the prompt itself can show a
/// value another shell just set; and again after a line has been read and before it runs, which is
/// the one that matters most — a shell sitting in its line editor has not reached the top of its
/// loop since before you typed, so without the second check the command you just ran would see the
/// value as it was one command ago.
pub fn refresh(env: &mut crate::env::Environment, stamp: Option<Stamp>) -> Option<Stamp> {
    let now = changed_at();
    if now != stamp {
        apply(env);
    }
    now
}

/// Everything in the file right now, or an empty map when there is no file.
pub fn load() -> BTreeMap<String, Universal> {
    let Some(path) = path() else {
        return BTreeMap::new();
    };
    match std::fs::read_to_string(&path) {
        Ok(text) => parse(&text),
        Err(_) => BTreeMap::new(),
    }
}

/// What the file looked like, for deciding whether a reload is needed.
///
/// **Three fields rather than a modification time.** A timestamp alone answers "has this changed?"
/// with a maybe: two writes that land in the same tick are one timestamp, and on a filesystem whose
/// `mtime` granularity is coarse — an old ext3, some network mounts — that is a whole second of
/// changes a reader cannot see. Linux gives nanoseconds on the filesystems oslo runs on, so this is
/// mostly belt and braces, but the belt costs nothing: `stat` already returns all three.
///
/// * `mtime` catches an ordinary in-place write.
/// * `len` catches a same-tick write that changed the length.
/// * `inode` catches a replacement — a `rename` over the file leaves both of the others plausible.
///
/// **And `inode` is the one that actually closes it**, because every write replaces the file rather
/// than editing it — see `update` below. A same-tick, same-length edit used to be invisible, and it is
/// not a theoretical case: `universal A=1` followed by `universal A=2` is two writes of identical
/// length, and inode timestamps are coarser than the gap between them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamp {
    mtime: Option<std::time::SystemTime>,
    len: u64,
    inode: u64,
}

/// One `stat` per prompt, which is what makes "see it everywhere" affordable: the file is only
/// read when it has actually moved.
pub fn changed_at() -> Option<Stamp> {
    use std::os::unix::fs::MetadataExt;
    let data = std::fs::metadata(path()?).ok()?;
    Some(Stamp {
        mtime: data.modified().ok(),
        len: data.len(),
        inode: data.ino(),
    })
}

/// The file the writers queue on. Never renamed, so every writer locks the same thing.
fn lock_path() -> Option<PathBuf> {
    Some(path()?.with_extension("lock"))
}

/// Apply `edit` to the file's contents under an exclusive lock.
///
/// The lock is held across the read *and* the write, which is what makes two shells editing
/// different variables at the same time safe. `Err` means the file could not be opened at all.
///
/// # Written through a rename, and the lock is a separate file because of it
///
/// An in-place rewrite — rewind, truncate, write — has two faults, and one of them is visible to
/// anybody. A reader takes no lock (that is the point: one `stat` and usually no read at all), so a
/// reader that lands between the truncate and the write sees an *empty store* and takes every
/// universal variable out of its environment. The other is that a same-length edit in the same
/// timestamp tick is invisible to [`Stamp`]: the file changed and nothing about it did.
///
/// Renaming a fresh file over the old one fixes both — the swap is atomic, so a reader sees one
/// version or the other and never a half of either, and the inode changes on every write, which is
/// the field the stamp can always trust. It costs the lock its home: the previous code locked the
/// data file itself, which a rename replaces, leaving the next writer holding a lock on a file
/// nobody else can reach. So the lock moves to a file that is only ever locked.
fn update(edit: impl FnOnce(&mut BTreeMap<String, Universal>)) -> Result<(), String> {
    let path =
        path().ok_or_else(|| "no home directory to store universal variables in".to_string())?;
    let lock = lock_path().ok_or_else(|| "no home directory".to_string())?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    let door = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock)
        .map_err(|e| format!("{}: {e}", lock.display()))?;

    // Exclusive and blocking: a competing shell is writing one variable, so the wait is the
    // length of one small file write, and losing the update would be the alternative.
    let _locked = nix::fcntl::Flock::lock(door, nix::fcntl::FlockArg::LockExclusive)
        .map_err(|(_, e)| format!("{}: cannot lock: {e}", lock.display()))?;

    let mut vars = parse(&std::fs::read_to_string(&path).unwrap_or_default());
    edit(&mut vars);

    // Beside the real file rather than in `/tmp`, because a rename is only atomic within one
    // filesystem — and a `$XDG_DATA_HOME` on a different mount is ordinary, not exotic.
    let staged = path.with_extension("new");
    let rendered = render(&vars);
    let mut file =
        std::fs::File::create(&staged).map_err(|e| format!("{}: {e}", staged.display()))?;
    file.write_all(rendered.as_bytes())
        .map_err(|e| format!("{}: {e}", staged.display()))?;
    file.flush()
        .map_err(|e| format!("{}: {e}", staged.display()))?;
    drop(file);
    std::fs::rename(&staged, &path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(())
}

/// Define or replace one.
pub fn set(name: &str, value: &str, exported: bool) -> Result<(), String> {
    update(|vars| {
        vars.insert(
            name.to_string(),
            Universal {
                value: value.to_string(),
                exported,
            },
        );
    })
}

/// Remove one. Answers whether it was there.
pub fn erase(name: &str) -> Result<bool, String> {
    let mut existed = false;
    update(|vars| existed = vars.remove(name).is_some())?;
    Ok(existed)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An isolated store, held for the duration of the test.
    fn isolated() -> (std::sync::MutexGuard<'static, ()>, tempfile::TempDir) {
        super::isolate_for_tests()
    }

    /// **A store that does not exist has not changed.**
    ///
    /// The servicer asks this on every idle wake. Confusing "no file" with "never looked" made the
    /// answer *yes* every time, which told the prompt to rebuild itself on every wake — and a
    /// prompt that spawns anything then woke the editor by exiting, three hundred times a second.
    #[test]
    fn a_store_that_is_not_there_does_not_keep_reporting_a_change() {
        let _store = isolated();
        let mut env = crate::env::Environment::new();

        assert!(
            apply_if_changed(&mut env),
            "the first look is a change: nothing had been applied yet"
        );
        for _ in 0..5 {
            assert!(
                !apply_if_changed(&mut env),
                "an absent store reported a change twice"
            );
        }

        // And a store that appears is noticed, once.
        set("APPEARED", "yes", false).expect("write");
        assert!(apply_if_changed(&mut env), "a new store went unnoticed");
        assert!(!apply_if_changed(&mut env), "and then it settled");
    }

    /// **Every write replaces the file**, which is what makes one field of the stamp trustworthy on
    /// its own.
    ///
    /// `universal A=1` then `universal A=2` renders two files of identical length, so `len` cannot
    /// tell them apart and only `mtime` is left — and inode timestamps are as coarse as the
    /// filesystem says, which on some is coarser than the gap between two commands. The `inode`
    /// check is the one that holds everywhere, and it holds only because the write is a rename.
    ///
    /// Asserted on the field rather than on the whole stamp deliberately: this filesystem has
    /// nanosecond timestamps, so comparing stamps would pass against an in-place write too and the
    /// test would be pinning nothing.
    #[test]
    fn every_write_replaces_the_file_rather_than_editing_it() {
        let _store = isolated();

        set("A", "1", false).expect("write");
        let before = changed_at().expect("a stamp");
        set("A", "2", false).expect("write");
        let after = changed_at().expect("a stamp");

        assert_eq!(load()["A"].value, "2");
        assert_ne!(
            before.inode, after.inode,
            "a same-length rewrite left the store looking untouched to any reader whose \
             filesystem timestamps are coarser than one command"
        );
    }

    /// **A reader never sees the store empty.**
    ///
    /// The old writer truncated the file and then filled it, and a reader in that window took every
    /// universal variable out of its environment — a wrong answer, not a stale one. The staged file
    /// is a separate name until the rename, so the only two things a reader can see are the old
    /// contents and the new.
    #[test]
    fn a_reader_during_a_write_sees_one_version_or_the_other() {
        let _store = isolated();
        set("KEEP", "yes", false).expect("write");

        let reading = std::thread::spawn(|| {
            let deadline = std::time::Instant::now() + std::time::Duration::from_millis(400);
            let mut empties = 0;
            while std::time::Instant::now() < deadline {
                if !load().contains_key("KEEP") {
                    empties += 1;
                }
            }
            empties
        });
        for n in 0..200 {
            set("CHURN", &format!("{n}"), false).expect("write");
        }
        assert_eq!(
            reading.join().expect("reader"),
            0,
            "a reader saw the store without a variable that was never erased"
        );
    }

    /// The file is written and read back through the real path, under the lock.
    #[test]
    fn a_variable_survives_being_written_and_read() {
        let _store = isolated();

        set("GREETING", "hello there", true).expect("write");
        set("SECOND", "x", false).expect("write");
        let vars = load();
        assert_eq!(vars["GREETING"].value, "hello there");
        assert!(vars["GREETING"].exported);
        assert!(!vars["SECOND"].exported);

        assert_eq!(erase("GREETING"), Ok(true));
        assert_eq!(erase("GREETING"), Ok(false), "already gone");
        let vars = load();
        assert!(!vars.contains_key("GREETING"));
        assert!(vars.contains_key("SECOND"), "erase touched one variable");
    }

    /// A universal variable fills in a name this shell has not set, and **loses** to one it has.
    ///
    /// The losing half is the one that matters: a universal `PAGER` must not make `PAGER=cat cmd`
    /// mean anything other than `cat`, or the variable is a lock rather than a default.
    #[test]
    fn a_local_assignment_beats_the_file() {
        let _store = isolated();

        set("UNIV_FILLED", "from-file", false).expect("write");
        set("UNIV_OWNED", "from-file", false).expect("write");

        let mut env = crate::env::Environment::new();
        env.set_var("UNIV_OWNED", "mine", false);
        apply(&mut env);

        assert_eq!(env.get_var("UNIV_FILLED"), Some("from-file"), "unset here");
        assert_eq!(env.get_var("UNIV_OWNED"), Some("mine"), "assigned here");
    }

    /// Erasing elsewhere takes the value away here — but only the value this shell put there.
    #[test]
    fn an_erase_elsewhere_removes_only_what_we_applied() {
        let _store = isolated();

        set("UNIV_DROPPED", "v", false).expect("write");
        let mut env = crate::env::Environment::new();
        apply(&mut env);
        assert_eq!(env.get_var("UNIV_DROPPED"), Some("v"));

        // The user then assigns over it, and *afterwards* it is erased elsewhere. Their value
        // stays: the file no longer has an opinion, and this shell's own assignment is not the
        // file's to withdraw.
        env.set_var("UNIV_DROPPED", "mine now", false);
        assert_eq!(erase("UNIV_DROPPED"), Ok(true));
        apply(&mut env);
        assert_eq!(env.get_var("UNIV_DROPPED"), Some("mine now"));
    }

    /// The plain case: applied by us, erased elsewhere, gone from here.
    #[test]
    fn an_untouched_value_is_withdrawn_when_the_file_drops_it() {
        let _store = isolated();

        set("UNIV_TEMP", "here", false).expect("write");
        let mut env = crate::env::Environment::new();
        apply(&mut env);
        assert_eq!(env.get_var("UNIV_TEMP"), Some("here"));

        assert_eq!(erase("UNIV_TEMP"), Ok(true));
        apply(&mut env);
        assert_eq!(env.get_var("UNIV_TEMP"), None, "erased elsewhere");
    }
}

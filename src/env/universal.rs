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

use std::collections::BTreeMap;
use std::io::{Read, Seek, Write};
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

/// Encode a value so it cannot span lines.
///
/// Only three characters need it, and everything else is written as itself — which keeps the file
/// readable, and readable is the point of choosing a text format at all.
fn escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            other => out.push(other),
        }
    }
    out
}

/// The inverse. An unknown escape keeps both characters rather than being dropped: losing data
/// silently is worse than a value that looks slightly wrong and can be corrected.
fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('\\') => out.push('\\'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// Parse the whole file. A line that makes no sense is skipped rather than failing the read —
/// one corrupt line must not cost you every other variable.
fn parse(text: &str) -> BTreeMap<String, Universal> {
    let mut out = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((flags, rest)) = line.split_once(' ') else {
            continue;
        };
        let Some((name, value)) = rest.split_once('=') else {
            continue;
        };
        if name.is_empty() {
            continue;
        }
        out.insert(
            name.to_string(),
            Universal {
                value: unescape(value),
                exported: flags.contains('x'),
            },
        );
    }
    out
}

fn render(vars: &BTreeMap<String, Universal>) -> String {
    let mut out = String::new();
    // A header, because somebody will find this file and wonder. It is skipped on read like any
    // other comment.
    out.push_str("# oslo universal variables — written by `universal`, do not source\n");
    for (name, var) in vars {
        let flag = if var.exported { 'x' } else { '-' };
        out.push_str(&format!("{flag} {name}={}\n", escape(&var.value)));
    }
    out
}

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

/// Apply every universal variable to `env`, and take away the ones that have been erased.
///
/// **An assignment in this shell wins.** A universal variable is a default that follows you
/// around, not a lock: `PAGER=cat some-command` has to mean `cat` for that command even if a
/// universal `PAGER` exists, and a shell that reversed that would be one nobody could override.
///
/// Telling the two apart is what the record above is for. A name whose current value is still
/// exactly what this shell last wrote is ours to change; a name the user has since assigned to is
/// not, and is left alone in both directions — not updated, and not removed when the file drops it.
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
        env.set_var(&name, &var.value, var.exported);
        note_applied(&name, &var.value);
    }
    for (name, last) in gone {
        if env.get_var(&name).is_some_and(|current| current == last) {
            env.unset_var(&name);
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
pub fn refresh(
    env: &mut crate::env::Environment,
    stamp: Option<std::time::SystemTime>,
) -> Option<std::time::SystemTime> {
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

/// When the file last changed, for deciding whether a reload is needed.
///
/// One `stat` per prompt, which is what makes "see it everywhere" affordable: the file is only
/// read when it has actually moved.
pub fn changed_at() -> Option<std::time::SystemTime> {
    std::fs::metadata(path()?).ok()?.modified().ok()
}

/// Apply `edit` to the file's contents under an exclusive lock.
///
/// The lock is held across the read *and* the write, which is what makes two shells editing
/// different variables at the same time safe. `Err` means the file could not be opened at all.
fn update(edit: impl FnOnce(&mut BTreeMap<String, Universal>)) -> Result<(), String> {
    let path =
        path().ok_or_else(|| "no home directory to store universal variables in".to_string())?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
    }
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .map_err(|e| format!("{}: {e}", path.display()))?;

    // Exclusive and blocking: a competing shell is writing one variable, so the wait is the
    // length of one small file write, and losing the update would be the alternative.
    let locked = nix::fcntl::Flock::lock(file, nix::fcntl::FlockArg::LockExclusive)
        .map_err(|(_, e)| format!("{}: cannot lock: {e}", path.display()))?;
    let mut file = locked;

    let mut text = String::new();
    file.read_to_string(&mut text)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let mut vars = parse(&text);
    edit(&mut vars);

    // Rewritten in place rather than through a temporary and a rename: the rename would replace
    // the file the lock is held on, and the next writer would lock a file nobody else can see.
    let rendered = render(&vars);
    file.rewind()
        .map_err(|e| format!("{}: {e}", path.display()))?;
    file.set_len(0)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    file.write_all(rendered.as_bytes())
        .map_err(|e| format!("{}: {e}", path.display()))?;
    file.flush()
        .map_err(|e| format!("{}: {e}", path.display()))?;
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

    /// A value containing the one character that would otherwise end its line survives.
    #[test]
    fn a_newline_in_a_value_does_not_end_the_line() {
        let mut vars = BTreeMap::new();
        vars.insert(
            "MULTI".to_string(),
            Universal {
                value: "one\ntwo\\three".to_string(),
                exported: false,
            },
        );
        let text = render(&vars);
        assert_eq!(
            text.lines().count(),
            2,
            "header plus one variable: {text:?}"
        );
        assert_eq!(parse(&text), vars, "must survive the round trip");
    }

    /// The export flag is part of the record, not something inferred on read.
    #[test]
    fn the_export_flag_round_trips() {
        let mut vars = BTreeMap::new();
        vars.insert(
            "SEEN".to_string(),
            Universal {
                value: "1".to_string(),
                exported: true,
            },
        );
        vars.insert(
            "HIDDEN".to_string(),
            Universal {
                value: "2".to_string(),
                exported: false,
            },
        );
        let back = parse(&render(&vars));
        assert!(back["SEEN"].exported);
        assert!(!back["HIDDEN"].exported);
    }

    /// One unreadable line costs that line and nothing else. A file this is rewritten into by
    /// every shell you have open must not become all-or-nothing.
    #[test]
    fn a_corrupt_line_does_not_cost_the_others() {
        let text = "# comment\n\
                    x GOOD=yes\n\
                    this line has no equals\n\
                    \n\
                    - ALSO=fine\n";
        let vars = parse(text);
        assert_eq!(vars.len(), 2, "{vars:?}");
        assert_eq!(vars["GOOD"].value, "yes");
        assert_eq!(vars["ALSO"].value, "fine");
    }

    /// An empty value is a value, and is different from the variable being absent.
    #[test]
    fn an_empty_value_is_kept() {
        let vars = parse("- EMPTY=\n");
        assert_eq!(vars["EMPTY"].value, "");
    }

    /// A value with an `=` in it keeps all of it — the split is at the *first* one.
    #[test]
    fn only_the_first_equals_separates() {
        let vars = parse("- URL=k=v&a=b\n");
        assert_eq!(vars["URL"].value, "k=v&a=b");
    }

    /// An escape nobody wrote is left alone rather than swallowed.
    #[test]
    fn an_unknown_escape_keeps_both_characters() {
        assert_eq!(unescape(r"a\qb"), r"a\qb");
        assert_eq!(unescape(r"trailing\"), r"trailing\");
        assert_eq!(unescape(r"\\"), r"\");
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

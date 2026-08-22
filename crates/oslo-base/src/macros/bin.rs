//! The stored scripts, written out as files for everything that is not oslo.
//!
//! ```text
//! ~/.local/sbin/git-rel     one file per stored script, 0755
//! ~/.local/sbin/.oslo       the manifest: what oslo put here, so it can take it away again
//! ```
//!
//! # Why files at all, when the database is the point
//!
//! Because a stored script is reachable *by name* only from a shell that can read the database, and
//! scripts call each other. `git-rel` runs under bash and calls `veri`; a tmux pane starts `inter`;
//! a `.desktop` file launches `appy`. None of those can ask oslo anything. Without a file on `$PATH`
//! the database can only ever be the second copy, with the real one still in a dotfiles directory.
//!
//! So the files are **derived**, the way [`mod@super::snapshot`] is derived: written by every mutation,
//! never edited, and safe to delete — the next `oslo macros` command writes them again.
//!
//! # oslo does not look here
//!
//! `env::builtins::spawn::resolve_program` skips this directory, so inside oslo a stored macro is
//! answered from the database and not from the copy. That keeps one promise in each direction: the
//! database is canonical for oslo, and **a real program on `$PATH` still wins over a stored macro**,
//! because the rule that a database row may not quietly redefine `test` is about files somebody else
//! installed — not about oslo's own output.
//!
//! Somebody who only ever uses oslo can delete the whole directory and lose nothing.
//!
//! # Scripts, not functions
//!
//! A function's whole point is that it runs *in* the calling shell — `mkcd` that cannot change your
//! directory is not the thing you wrote. That cannot be a file, so functions are not written here
//! and calling one from bash is not something this pretends to offer.

use super::{Entry, Kind};
use std::io::Write;
use std::path::{Path, PathBuf};

/// The manifest, named so it sorts out of the way and reads as oslo's.
const MANIFEST: &str = ".oslo";

/// `$OSLO_MACROS_BIN`, or `~/.local/sbin`.
///
/// `sbin` rather than `bin` on purpose: `~/.local/bin` is where a person puts their own scripts, and
/// a directory oslo rewrites on every change is no place to keep something hand-written.
pub fn directory() -> Option<PathBuf> {
    if let Some(named) = std::env::var_os("OSLO_MACROS_BIN")
        && !named.is_empty()
    {
        return Some(PathBuf::from(named));
    }
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join(".local/sbin"))
}

/// Whether `path` is a file this module wrote.
///
/// **The manifest decides, not the directory.** Taking "in that directory" to mean "ours" is one
/// string comparison and it is wrong about anything a person drops in there by hand: that file
/// would run from bash and from a `.desktop` entry, and be *not found* in the shell whose directory
/// it is. Nothing else on the system behaves that way, and there is a list of what oslo wrote
/// sitting right beside the files.
///
/// Asked by the command resolver on a hit, so the cheap half comes first: a name that resolved
/// anywhere else is answered by comparing two paths, and the manifest is read only for a hit inside
/// oslo's own directory.
pub fn is_ours(path: &Path) -> bool {
    let (Some(parent), Some(dir), Some(name)) = (path.parent(), directory(), path.file_name())
    else {
        return false;
    };
    parent == dir
        && read_manifest(&dir)
            .iter()
            .any(|written| std::ffi::OsStr::new(written) == name)
}

/// Write every stored script out, and take away the ones that are no longer stored.
///
/// Errors are answered rather than raised: this runs from every mutation, and a `oslo macros add`
/// that failed because a directory could not be created would be reporting the wrong problem. The
/// caller decides whether anybody needs to hear about it.
pub fn publish(entries: &[Entry]) -> Result<(), String> {
    let Some(dir) = directory() else {
        return Err("no $HOME, so there is nowhere to write the scripts".to_string());
    };
    // **A directory belongs to the store that filled it.** Two stores, one directory, and the
    // second one's publish deletes the first one's scripts — which is not a hypothetical: pointing
    // `$XDG_DATA_HOME` at a scratch store and running `oslo macros publish` emptied a real
    // `~/.local/sbin` of seventy-two files, because the store moved and this directory did not.
    // The manifest says whose it is, so the answer is to look before writing.
    if let Some(owner) = owner_of(&dir)
        && Some(&owner) != super::directory().as_ref()
    {
        return Err(format!(
            "{} holds scripts published from {} — set $OSLO_MACROS_BIN to write somewhere else",
            dir.display(),
            owner.display()
        ));
    }
    let wanted: Vec<&Entry> = entries
        .iter()
        .filter(|entry| entry.kind == Kind::Script && entry.active)
        .collect();

    // Nothing stored and nothing written before: do not create a directory to keep empty.
    let written = read_manifest(&dir);
    if wanted.is_empty() && written.is_empty() {
        return Ok(());
    }
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;

    // **A file oslo did not write is somebody else's on the way *in* as well as on the way out.**
    // The removal loop below has always said so; this one did not, so publishing a script called
    // `deploy` replaced a hand-placed `deploy` in this directory without a word. The manifest is
    // the same answer to the same question, asked one step earlier.
    let mut refused: Vec<String> = Vec::new();
    let mut now: Vec<String> = Vec::new();
    for entry in &wanted {
        let target = dir.join(&entry.name);
        if target.exists() && !written.contains(&entry.name) {
            refused.push(entry.name.clone());
            continue;
        }
        write_one(&target, &entry.body)?;
        now.push(entry.name.clone());
    }

    // **Only what oslo wrote**, which is what the manifest is for. A file in this directory that
    // oslo did not put there is somebody else's and is left alone, even though the directory is
    // meant to be oslo's — the cost of being wrong about that is deleting a person's script.
    for gone in written.iter().filter(|name| !now.contains(name)) {
        let _ = std::fs::remove_file(dir.join(gone));
    }
    write_manifest(&dir, &now)?;
    // Reported after everything that *could* be written has been, so one refusal does not cost the
    // rest of the publish. The script is stored either way; what it does not have is a file.
    if refused.is_empty() {
        return Ok(());
    }
    Err(format!(
        "{} already holds {} that oslo did not write, so {} not published — rename yours, or move that file",
        dir.display(),
        refused.join(", "),
        if refused.len() == 1 {
            "it was"
        } else {
            "they were"
        }
    ))
}

/// One script, executable, replaced through a rename so nothing ever reads half of it.
fn write_one(path: &Path, body: &str) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let scratch = path.with_extension("oslo-new");
    let mut file = std::fs::File::create(&scratch)
        .map_err(|e| format!("{}: {}", scratch.display(), crate::error::reason(&e)))?;
    file.write_all(body.as_bytes())
        .map_err(|e| format!("{}: {}", scratch.display(), crate::error::reason(&e)))?;
    // Executable by its owner and readable by nobody else, which is what a private script is. The
    // database it came from is `0600` and copying it out should not widen it.
    let _ = file.set_permissions(std::fs::Permissions::from_mode(0o700));
    drop(file);
    std::fs::rename(&scratch, path)
        .map_err(|e| format!("{}: {}", path.display(), crate::error::reason(&e)))
}

/// The names oslo wrote into `dir`, from its manifest.
///
/// The `store …` line is not one of them: it says *whose* the directory is, and is read by
/// [`owner_of`].
fn read_manifest(dir: &Path) -> Vec<String> {
    std::fs::read_to_string(dir.join(MANIFEST))
        .map(|text| {
            text.lines()
                .map(str::trim)
                .filter(|line| !line.is_empty() && !line.starts_with(OWNER))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// The first line of a manifest, naming the store the files were published from.
const OWNER: &str = "store ";

/// Which store filled this directory, when it says.
///
/// `None` for a directory with no manifest, and for one written before this line existed — in both
/// cases nobody has claimed it and the caller may take it.
fn owner_of(dir: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(dir.join(MANIFEST)).ok()?;
    let line = text.lines().find(|line| line.starts_with(OWNER))?;
    Some(PathBuf::from(line[OWNER.len()..].trim()))
}

fn write_manifest(dir: &Path, names: &[String]) -> Result<(), String> {
    let path = dir.join(MANIFEST);
    let mut text = String::new();
    if let Some(store) = super::directory() {
        text.push_str(&format!("{OWNER}{}\n", store.display()));
    }
    text.push_str(&names.join("\n"));
    if !names.is_empty() {
        text.push('\n');
    }
    std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
#[path = "bin/tests.rs"]
mod tests;

//! What a command was asked to keep, so `copy --last` has something to copy.
//!
//! # Opt-in, and why it has to be
//!
//! A shell that kept every command's output would hold all of `cargo build` to answer a question
//! nobody asked, and it could not do it honestly either: to see the bytes it has to sit between the
//! command and the terminal, and a pipe in that gap turns `isatty` false — colours off, pagers
//! changed, progress bars silent — for every command you run. So `keep make build` is a decision
//! taken one command at a time, and everything else runs exactly as it did.
//!
//! # A file, not a variable in the shell
//!
//! `keep` in a pipeline runs in a forked child, and anything it remembered in memory would die with
//! that child. The file outlives the fork, which also means `copy --last` works from a hook, from a
//! subshell, and from the other pane of the same session.
//!
//! # One per session
//!
//! Two terminals are two shells, and "the last output" means a different thing in each of them.
//! Files are named by [`crate::track::session::id`], which is the same name the terminal
//! integration marks commands with, and stale ones are swept the way the macro store sweeps its
//! own.

use std::path::PathBuf;

/// The most text a single capture keeps: 1 MiB.
///
/// Not a limit on what the command may print — everything is shown as it arrives, and only what is
/// *kept* is bounded. A build log is unbounded and a clipboard is not.
pub const MAX: usize = 1024 * 1024;

/// A kept capture older than this is nobody's "last output" any more.
const STALE_SECONDS: u64 = 24 * 60 * 60;

/// `$XDG_DATA_HOME/oslo/capture`, or `~/.local/share/oslo/capture`.
pub fn directory() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
        .or_else(|| {
            std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share"))
        })?;
    Some(base.join("oslo/capture"))
}

/// Where session `id` keeps its last output.
pub fn path(id: &str) -> Option<PathBuf> {
    Some(directory()?.join(format!("{id}.out")))
}

/// Keep `text` as session `id`'s last output; `true` when it had to be trimmed to fit [`MAX`].
///
/// The **tail** is what survives a trim. The end of a long output is the part worth having — the
/// error the build stopped at, the last page of a log — and the beginning is what you already read.
pub fn store(id: &str, text: &str) -> Result<bool, String> {
    let Some(path) = path(id) else {
        return Err(
            "no $XDG_DATA_HOME and no $HOME, so there is nowhere to keep output".to_string(),
        );
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        sweep(parent, &path);
    }
    let (kept, trimmed) = trim(text);
    std::fs::write(&path, kept).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(trimmed)
}

/// What session `id` last kept, or `None` when it has kept nothing.
pub fn last(id: &str) -> Option<String> {
    std::fs::read_to_string(path(id)?).ok()
}

/// Forget what this session kept.
pub fn forget(id: &str) {
    if let Some(path) = path(id) {
        let _ = std::fs::remove_file(path);
    }
}

/// The last [`MAX`] bytes of `text`, cut at a character boundary, and whether anything was cut.
fn trim(text: &str) -> (&str, bool) {
    if text.len() <= MAX {
        return (text, false);
    }
    let from = text.len() - MAX;
    // Cutting mid-character would make the file invalid UTF-8, so the cut moves forward to the
    // next boundary rather than being taken literally.
    let start = (from..text.len())
        .find(|at| text.is_char_boundary(*at))
        .unwrap_or(text.len());
    (&text[start..], true)
}

/// Remove captures no running shell will ask for again.
///
/// Never this session's own file, whatever its timestamp says — a clock that moved backwards is not
/// a reason to throw away the output somebody is about to copy.
fn sweep(directory: &std::path::Path, mine: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.path() == mine {
            continue;
        }
        let old = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .map(|when| when.elapsed().map(|since| since.as_secs()).unwrap_or(0) > STALE_SECONDS)
            .unwrap_or(false);
        if old {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
#[path = "capture/tests.rs"]
mod tests;

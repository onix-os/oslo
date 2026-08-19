//! What a repository says about itself, read from `.git` rather than from `git`.
//!
//! # Why not run git
//!
//! A prompt draws on every keystroke that changes the line, and `git` is a process: fork, exec,
//! read a config, open the object database, answer, exit. The `nix` segment shells out and costs
//! 6 ms, measured, which is why [`crate::prompt`] grew a cache and why `oslo.spawn` exists at all.
//! Everything here is one to three small file reads and no process.
//!
//! # What is here, and what is deliberately not
//!
//! Everything below is a *file lookup*: which ref `HEAD` names, whether a sentinel file exists, how
//! many lines a log has. Those are exact, cheap and cannot disagree with git.
//!
//! **`dirty` and `ahead`/`behind` are not here**, and their absence is the design rather than an
//! omission. A dirty check means comparing the index against every tracked file in the working
//! tree; ahead/behind means walking commit history through the object database, packfiles and
//! deltas included. Both are real work, both are exactly what git is good at, and a wrong answer
//! from a hand-rolled version is worse than no answer. The shape for those is the one the docs
//! already recommend:
//!
//! ```lua
//! oslo.every(2000, function()
//!   oslo.spawn{ "git", "status", "--porcelain",
//!     on_exit = function(out) oslo.state.set("git.dirty", out ~= "") end }
//! end)
//! ```
//!
//! — asked off the prompt, answered into `oslo.state`, drawn from there.

use std::fs;
use std::path::{Path, PathBuf};

/// The real git directory for the working tree the shell is standing in.
///
/// **`.git` is not always a directory.** In a linked worktree and in a submodule it is a *file*
/// holding `gitdir: /path/to/the/real/one`, and everything here would otherwise be reading paths
/// under a regular file and answering `nil` for a repository that is plainly there.
pub fn dir() -> Option<PathBuf> {
    let dot = super::prompt::git_root()?.join(".git");
    if dot.is_dir() {
        return Some(dot);
    }
    let pointer = fs::read_to_string(&dot).ok()?;
    let target = pointer.trim().strip_prefix("gitdir:")?.trim();
    let path = Path::new(target);
    // A relative `gitdir:` is relative to the file holding it.
    Some(if path.is_absolute() {
        path.to_path_buf()
    } else {
        dot.parent()?.join(path)
    })
}

/// The directory holding what every worktree of this repository *shares*.
///
/// **A linked worktree's git directory holds almost nothing.** `HEAD`, the index and the sentinels
/// for an operation in progress are per-worktree; the refs, `packed-refs`, `config` and the stash
/// reflog all live in the main repository, and git records where that is in a `commondir` file next
/// to them. Reading a branch's commit from the worktree's own directory finds nothing there — which
/// is why `oslo.git.head().commit` was `nil` inside one until this existed.
///
/// The same directory when there is no linking, so callers do not have to ask which case they are
/// in.
pub fn common_dir() -> Option<PathBuf> {
    let git = dir()?;
    let Ok(pointer) = fs::read_to_string(git.join("commondir")) else {
        return Some(git);
    };
    let target = Path::new(pointer.trim());
    Some(if target.is_absolute() {
        target.to_path_buf()
    } else {
        // Relative to the worktree's own git directory, and worth normalising: the usual value is
        // `../..`, and a path with those left in reads badly in a message.
        normalise(&git.join(target))
    })
}

/// Resolve `.` and `..` without touching the disk, so a joined `commondir` reads as a path.
fn normalise(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

/// Where `HEAD` is: on a branch, or at a commit with no branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Head {
    /// The branch's short name, or `None` when detached.
    pub branch: Option<String>,
    /// The full object id, when it can be resolved.
    pub commit: Option<String>,
}

impl Head {
    pub fn detached(&self) -> bool {
        self.branch.is_none()
    }
}

/// What `HEAD` names, or `None` outside a repository.
pub fn head() -> Option<Head> {
    let git = dir()?;
    let content = fs::read_to_string(git.join("HEAD")).ok()?;
    let trimmed = content.trim();
    match trimmed.strip_prefix("ref: ") {
        Some(reference) => Some(Head {
            branch: Some(
                reference
                    .strip_prefix("refs/heads/")
                    .unwrap_or(reference)
                    .to_string(),
            ),
            // A branch with no commits yet — right after `git init` — has a `HEAD` naming a ref
            // that does not exist. That is a real state and `nil` is the honest answer for it.
            commit: resolve(&git, reference),
        }),
        // Detached: `HEAD` holds the object id itself.
        None if is_object_id(trimmed) => Some(Head {
            branch: None,
            commit: Some(trimmed.to_string()),
        }),
        None => None,
    }
}

/// The object id a ref names, following the loose file first and `packed-refs` after.
///
/// **In that order, because that is git's own.** A ref exists in both after `git gc` runs and the
/// branch then moves; the loose file is the newer one, and reading `packed-refs` first would report
/// a commit the branch left behind — which is exactly the kind of wrong that looks right.
///
/// `git` is the worktree's own directory and is tried first, because a few refs — `refs/bisect/*`
/// among them — are per-worktree; everything else is found in [`common_dir`].
pub fn resolve(git: &Path, reference: &str) -> Option<String> {
    let common = fs::read_to_string(git.join("commondir"))
        .ok()
        .map(|pointer| normalise(&git.join(pointer.trim())));
    for at in [Some(git.to_path_buf()), common].into_iter().flatten() {
        if let Ok(loose) = fs::read_to_string(at.join(reference)) {
            let id = loose.trim();
            if is_object_id(id) {
                return Some(id.to_string());
            }
        }
        if let Some(found) = packed(&at, reference) {
            return Some(found);
        }
    }
    None
}

/// A ref's object id from `packed-refs`.
fn packed(git: &Path, reference: &str) -> Option<String> {
    let packed = fs::read_to_string(git.join("packed-refs")).ok()?;
    packed.lines().find_map(|line| {
        // `^<id>` lines are the peeled target of the tag above them, not a ref of their own.
        if line.starts_with('#') || line.starts_with('^') {
            return None;
        }
        let (id, name) = line.split_once(' ')?;
        (name.trim() == reference && is_object_id(id)).then(|| id.to_string())
    })
}

/// Whether `text` looks like an object id, rather than a ref name or a stray line.
fn is_object_id(text: &str) -> bool {
    // 40 for sha-1 and 64 for sha-256, which git has supported since 2.29.
    matches!(text.len(), 40 | 64) && text.bytes().all(|b| b.is_ascii_hexdigit())
}

/// A multi-step git operation that is part-way through, if one is.
///
/// **Worth knowing because it changes what the next command means.** A prompt that says `main` when
/// the repository is half-way through a rebase is telling you something untrue about where you are,
/// and the sentinel files git leaves behind are the same ones its own shell prompt reads.
pub fn operation() -> Option<&'static str> {
    let git = dir()?;
    // Order matters where two can be true: a rebase that stops on a conflict leaves `MERGE_HEAD`
    // as well, and the rebase is the thing you are in the middle of.
    for (name, marker) in [
        ("rebase", "rebase-merge"),
        ("rebase", "rebase-apply"),
        ("merge", "MERGE_HEAD"),
        ("cherry-pick", "CHERRY_PICK_HEAD"),
        ("revert", "REVERT_HEAD"),
        ("bisect", "BISECT_LOG"),
    ] {
        if git.join(marker).exists() {
            return Some(name);
        }
    }
    None
}

/// How many entries the stash holds.
///
/// The reflog is the stash: one line per entry, newest first. No log means nothing stashed, which
/// is `0` rather than an error.
pub fn stash_count() -> usize {
    let Some(git) = common_dir() else {
        return 0;
    };
    fs::read_to_string(git.join("logs/refs/stash"))
        .map(|log| log.lines().filter(|line| !line.trim().is_empty()).count())
        .unwrap_or(0)
}

/// The branch `HEAD` tracks, as `origin/main`, or `None` when it tracks nothing.
///
/// Read from `.git/config`, which is where `git branch --set-upstream-to` writes it.
pub fn upstream() -> Option<String> {
    // `config` is shared by every worktree, so the common directory rather than this one's.
    let git = common_dir()?;
    let branch = head()?.branch?;
    let config = fs::read_to_string(git.join("config")).ok()?;
    let section = section_of(&config, &format!("branch \"{branch}\""))?;
    let remote = setting(&section, "remote")?;
    let merge = setting(&section, "merge")?;
    let short = merge.strip_prefix("refs/heads/").unwrap_or(&merge);
    // A `remote` of `.` means the branch tracks another branch in this same repository; naming it
    // `./main` would be noise, so the branch alone is the answer.
    Some(if remote == "." {
        short.to_string()
    } else {
        format!("{remote}/{short}")
    })
}

/// A tag pointing at `HEAD`, if any. The first by name, so the answer is stable.
pub fn tag_at_head() -> Option<String> {
    // Tags are shared too.
    let git = common_dir()?;
    let at = head()?.commit?;

    let mut found: Vec<String> = Vec::new();
    if let Ok(entries) = fs::read_dir(git.join("refs/tags")) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if resolve(&git, &format!("refs/tags/{name}")).as_deref() == Some(at.as_str()) {
                found.push(name);
            }
        }
    }
    if let Ok(packed) = fs::read_to_string(git.join("packed-refs")) {
        for line in packed.lines() {
            if line.starts_with('#') || line.starts_with('^') {
                continue;
            }
            if let Some((id, name)) = line.split_once(' ')
                && id == at
                && let Some(tag) = name.trim().strip_prefix("refs/tags/")
            {
                found.push(tag.to_string());
            }
        }
    }
    found.sort();
    found.dedup();
    found.into_iter().next()
}

/// The body of `[name]` in an INI-shaped config, up to the next section header.
fn section_of(config: &str, name: &str) -> Option<String> {
    let mut inside = false;
    let mut body = String::new();
    for line in config.lines() {
        let trimmed = line.trim();
        if let Some(header) = trimmed.strip_prefix('[').and_then(|t| t.strip_suffix(']')) {
            if inside {
                break;
            }
            inside = header.trim() == name;
            continue;
        }
        if inside {
            body.push_str(line);
            body.push('\n');
        }
    }
    inside.then_some(body)
}

/// `key = value` from a section body.
fn setting(section: &str, key: &str) -> Option<String> {
    section.lines().find_map(|line| {
        let (name, value) = line.split_once('=')?;
        (name.trim() == key).then(|| value.trim().to_string())
    })
}

#[cfg(test)]
#[path = "git/tests.rs"]
mod tests;

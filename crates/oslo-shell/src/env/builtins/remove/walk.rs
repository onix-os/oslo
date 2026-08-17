//! Removing an operand one entry at a time.
//!
//! # Why not `remove_dir_all`
//!
//! `std::fs::remove_dir_all` is one call and answers one error, and both of those are wrong for
//! `rm`. A tree with a single unreadable file underneath it came back as
//!
//! ```text
//! oslo: rm: cannot remove 'noperm': Permission denied
//! ```
//!
//! which names a directory that is perfectly readable, and stops — leaving every entry it had not
//! reached yet in place. `rm -rf noperm` on GNU names `noperm/inner/f`, removes everything else,
//! and exits 1. The difference matters most in exactly the case people reach for `rm -rf`: a build
//! tree with one root-owned file in it, where "it failed" and "it failed *here*, and the rest is
//! gone" are different problems.
//!
//! So the walk is oslo's own: it reports the path that actually failed, carries on to the entries
//! that can still go, and prints a `-v` line per entry rather than one for the whole tree.
//!
//! # Three things the walk has to get right
//!
//! * **A symlink is one entry.** Descending into what it points at is how `rm -r` deletes a home
//!   directory; `symlink_metadata` per entry is what stops it, and `is_dir()` on that is false for
//!   a link to a directory.
//! * **An explicit stack, not recursion.** Depth is whatever the filesystem holds, and a tree deep
//!   enough to overflow the Rust stack is a tree someone can build on purpose.
//! * **A parent whose child failed says nothing.** It cannot be removed either, but the `ENOTEMPTY`
//!   is a consequence of a failure already reported, and printing one per level buries the line
//!   that says what is actually wrong.

use oslo_base::error::reason;
use std::fs::Metadata;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};

/// How one operand should be taken apart.
pub struct Walk {
    /// Where a diagnostic says it came from — `env.origin()`, so a failure inside a script names
    /// the script and the line rather than the shell.
    pub origin: String,
    /// `-f`: missing entries are not errors, and nothing is ever prompted for.
    pub force: bool,
    /// `-i`: prompt before each entry, and before descending into each directory.
    pub interactive: bool,
    /// `-r`: descend. Without it a directory operand is removed only if it is empty.
    pub recursive: bool,
    /// `-v`: name each entry as it goes.
    pub verbose: bool,
}

/// What became of an operand.
pub struct Outcome {
    /// Whether anything under it could not be removed.
    pub failed: bool,
    /// Whether a Ctrl-C stopped the walk part-way.
    pub interrupted: bool,
}

/// One item of work. `Leave` is pushed under a directory's children so the directory itself is
/// removed after them, carrying the failure count from when it was entered.
enum Step {
    Enter(PathBuf, String),
    Leave(PathBuf, String, usize),
}

/// Remove `root`, and everything under it when `recursive`.
pub fn remove_tree(root: &Path, shown: &str, walk: &Walk) -> Outcome {
    let mut failures = 0usize;
    let mut stack = vec![Step::Enter(root.to_path_buf(), shown.to_string())];

    while let Some(step) = stack.pop() {
        // Between entries, which is the only place a builtin can be stopped: it runs in the shell
        // process, so the keystroke sets a flag that nothing would otherwise look at until the
        // whole `rm` had finished. Peeked rather than taken — see `job::interrupt_waiting`.
        if crate::exec::job::interrupt_waiting() {
            return Outcome {
                failed: failures > 0,
                interrupted: true,
            };
        }
        match step {
            Step::Enter(path, shown) => {
                let Some(meta) = look(&path, &shown, walk, &mut failures) else {
                    continue;
                };
                if meta.is_dir() {
                    descend(&mut stack, path, shown, walk, &mut failures);
                } else {
                    unlink(&path, &shown, &meta, walk, &mut failures);
                }
            }
            Step::Leave(path, shown, before) => {
                if failures > before {
                    continue;
                }
                if !ask(walk, &shown, "directory", &path) {
                    continue;
                }
                take(
                    std::fs::remove_dir(&path),
                    &shown,
                    true,
                    walk,
                    &mut failures,
                );
            }
        }
    }

    Outcome {
        failed: failures > 0,
        interrupted: false,
    }
}

/// The entry's own metadata, or `None` when it is not there to remove.
fn look(path: &Path, shown: &str, walk: &Walk, failures: &mut usize) -> Option<Metadata> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) => Some(meta),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound && walk.force => None,
        Err(e) => {
            complain(walk, shown, &e);
            *failures += 1;
            None
        }
    }
}

/// Queue a directory's children, then the directory itself.
fn descend(stack: &mut Vec<Step>, path: PathBuf, shown: String, walk: &Walk, failures: &mut usize) {
    if !walk.recursive {
        if !ask(walk, &shown, "directory", &path) {
            return;
        }
        take(std::fs::remove_dir(&path), &shown, true, walk, failures);
        return;
    }

    let children = match read_children(&path, &shown) {
        Ok(children) => children,
        Err(e) => {
            complain(walk, &shown, &e);
            *failures += 1;
            return;
        }
    };

    // The descend prompt is asked only when there is something to descend into; GNU asks an empty
    // directory the one question that applies to it, which `Leave` puts a few lines below.
    if !children.is_empty()
        && walk.interactive
        && !walk.force
        && !confirm(&walk.origin, &format!("descend into directory '{shown}'"))
    {
        return;
    }

    stack.push(Step::Leave(path, shown, *failures));
    // Reversed, so popping yields them in the order the directory listed them — the order `-v`
    // output and GNU's both come out in.
    for child in children.into_iter().rev() {
        stack.push(child);
    }
}

/// A directory's entries as steps, with the operand's spelling carried down the path.
fn read_children(path: &Path, shown: &str) -> std::io::Result<Vec<Step>> {
    let stem = shown.trim_end_matches('/');
    let mut children = Vec::new();
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let name = entry.file_name();
        children.push(Step::Enter(
            entry.path(),
            format!("{stem}/{}", name.to_string_lossy()),
        ));
    }
    Ok(children)
}

/// Remove one non-directory entry.
fn unlink(path: &Path, shown: &str, meta: &Metadata, walk: &Walk, failures: &mut usize) {
    if !ask(walk, shown, describe(meta), path) {
        return;
    }
    take(std::fs::remove_file(path), shown, false, walk, failures);
}

/// Record what a removal did, and say so under `-v`.
fn take(
    removed: std::io::Result<()>,
    shown: &str,
    directory: bool,
    walk: &Walk,
    failures: &mut usize,
) {
    match removed {
        Ok(()) => {
            if walk.verbose {
                if directory {
                    println!("removed directory '{shown}'");
                } else {
                    println!("removed '{shown}'");
                }
            }
        }
        Err(e) => {
            complain(walk, shown, &e);
            *failures += 1;
        }
    }
}

/// Whether this entry may go.
///
/// **`-f` never asks, `-i` always asks, and in between there is the write-protected prompt** — the
/// one GNU raises for a file the user cannot write to, and only when someone is at the terminal to
/// answer it. A script's stdin is not a tty, so this is silent everywhere it would otherwise hang.
fn ask(walk: &Walk, shown: &str, kind: &str, path: &Path) -> bool {
    if walk.force {
        return true;
    }
    if walk.interactive {
        return confirm(&walk.origin, &format!("remove {kind} '{shown}'"));
    }
    if !std::io::stdin().is_terminal() || writable(path) {
        return true;
    }
    confirm(
        &walk.origin,
        &format!("remove write-protected {kind} '{shown}'"),
    )
}

/// Whether the user could write to this path, which is what decides the extra prompt.
fn writable(path: &Path) -> bool {
    nix::unistd::access(path, nix::unistd::AccessFlags::W_OK).is_ok()
}

/// Anything but a `y` answer means no, as it does in `rm` and in `find -ok`.
pub fn confirm(origin: &str, question: &str) -> bool {
    eprint!("{origin}rm: {question}? ");
    let _ = std::io::Write::flush(&mut std::io::stderr());
    let mut answer = String::new();
    match std::io::BufRead::read_line(&mut std::io::stdin().lock(), &mut answer) {
        Ok(0) | Err(_) => false,
        Ok(_) => matches!(answer.trim_start().chars().next(), Some('y') | Some('Y')),
    }
}

fn complain(walk: &Walk, shown: &str, e: &std::io::Error) {
    eprintln!("{}rm: cannot remove '{shown}': {}", walk.origin, reason(e));
}

/// What `rm` calls this kind of entry when it asks about it.
///
/// GNU's wording, because a prompt is something people answer by reading it, and "regular empty
/// file" is the difference between deleting a stub and deleting a day's work.
pub fn describe(meta: &Metadata) -> &'static str {
    use std::os::unix::fs::FileTypeExt;
    let kind = meta.file_type();
    if kind.is_symlink() {
        "symbolic link"
    } else if kind.is_dir() {
        "directory"
    } else if kind.is_fifo() {
        "fifo"
    } else if kind.is_socket() {
        "socket"
    } else if kind.is_char_device() {
        "character special file"
    } else if kind.is_block_device() {
        "block special file"
    } else if meta.len() == 0 {
        "regular empty file"
    } else {
        "regular file"
    }
}

#[cfg(test)]
#[path = "walk/tests.rs"]
mod tests;

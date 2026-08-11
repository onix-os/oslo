//! Where scratches live, and proving it is ours before anything is written there.
//!
//! `/tmp/oslo-$UID/scratch`, `0700`. `/tmp` is chosen over `$XDG_RUNTIME_DIR` because systemd deletes
//! the latter at last logout unless lingering is enabled, and a scratch dying at logout defeats the
//! whole feature; `/tmp` survives it with nothing to configure. tmux made the same trade for the
//! same reason and has lived at `/tmp/tmux-$UID` for twenty years.
//!
//! # The check is the point of this module
//!
//! `/tmp` is `1777`, so **anybody can create `/tmp/oslo-1000` first and wait**. What would land in
//! it is a socket carrying a terminal's input and output, which is as bad as it sounds. So the
//! directory is proven before use: a real directory, our uid, mode `0700`, not a symlink — and the
//! check is made against an **open descriptor**, never against the path, because a path checked and
//! then opened is a path that can change in between.

use nix::fcntl::{OFlag, open};
use nix::sys::stat::{FileStat, Mode, fstat};
use nix::unistd::mkdir;
use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::path::{Path, PathBuf};

/// The mode a scratch directory must have, and is created with. Nothing shared, nothing readable.
const REQUIRED: u32 = 0o700;

/// Where scratches live: `$OSLO_SCRATCH_DIR`, or `/tmp/oslo-$UID/scratch`.
///
/// The variable is what `oslo.scratch.dir` writes, and what a test uses to stay out of the real one.
pub fn path() -> PathBuf {
    if let Some(set) = std::env::var_os("OSLO_SCRATCH_DIR").filter(|value| !value.is_empty()) {
        return PathBuf::from(set);
    }
    default_root().join("scratch")
}

/// `/tmp/oslo-$UID` — the directory oslo creates in a world-writable place, and therefore the one
/// it has to defend.
fn default_root() -> PathBuf {
    PathBuf::from(format!("/tmp/oslo-{}", nix::unistd::getuid().as_raw()))
}

/// The parent worth checking, which is only ever oslo's own.
///
/// **A configured directory's parent is not oslo's business.** The guard exists because
/// `/tmp/oslo-$UID` is a name a stranger can take first in a `1777` directory; somewhere you named
/// yourself is somewhere you chose, and demanding `0700` of *its* parent would refuse perfectly
/// reasonable places for no reason.
fn guarded_parent() -> Option<PathBuf> {
    (path() == default_root().join("scratch")).then(default_root)
}

/// Open the scratch directory, creating it if it does not exist, and refuse anything suspicious.
///
/// The returned descriptor is the only safe way to reach the contents: everything else in this
/// module works relative to it, so a directory swapped after the check cannot be reached through it.
pub fn open_checked() -> io::Result<OwnedFd> {
    let dir = path();
    // `/tmp/oslo-$UID` is the exposed name — the one a stranger can take first in a `1777`
    // directory. Proven ours and `0700`, nothing can reach inside it, so `scratches` within it needs
    // creating but not defending. It is still checked below, because the cost is one `fstat` and
    // the alternative is a rule with an exception in it.
    if let Some(parent) = guarded_parent() {
        make(&parent)?;
        let guard = open_dir(&parent)?;
        check(&stat_of(&guard)?, &parent)?;
    } else if let Some(parent) = dir.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    make(&dir)?;
    let fd = open_dir(&dir)?;
    check(&stat_of(&fd)?, &dir)?;
    Ok(fd)
}

/// Create `dir` with the required mode, and say nothing if it is already there.
///
/// **Created at `0700` rather than created and then chmod'ed.** The window between the two is a
/// window where the directory exists and is readable, which is the whole of what is being avoided.
fn make(dir: &Path) -> io::Result<()> {
    match mkdir(dir, Mode::from_bits_truncate(REQUIRED)) {
        Ok(()) => Ok(()),
        Err(nix::errno::Errno::EEXIST) => Ok(()),
        Err(err) => Err(io::Error::other(format!("{}: {err}", dir.display()))),
    }
}

/// Open a directory without following a final symlink.
fn open_dir(dir: &Path) -> io::Result<OwnedFd> {
    let raw = open(
        dir,
        OFlag::O_RDONLY | OFlag::O_DIRECTORY | OFlag::O_NOFOLLOW | OFlag::O_CLOEXEC,
        Mode::empty(),
    )
    .map_err(|err| io::Error::other(format!("{}: {err}", dir.display())))?;
    // SAFETY: `open` has just returned this descriptor and nothing else holds it, so taking
    // ownership here is what closes it exactly once.
    Ok(unsafe { OwnedFd::from_raw_fd(raw) })
}

/// The descriptor's own metadata — never the path's. See the module note.
fn stat_of(fd: &OwnedFd) -> io::Result<FileStat> {
    fstat(fd.as_raw_fd()).map_err(|err| io::Error::other(format!("fstat: {err}")))
}

/// Refuse a directory that is not ours, or is open to anybody else.
///
/// `O_DIRECTORY` and `O_NOFOLLOW` above have already refused a file and a symlink; this is the
/// ownership and the mode, and it reads the descriptor rather than the path.
fn check(stat: &FileStat, dir: &Path) -> io::Result<()> {
    let us = nix::unistd::getuid().as_raw();
    if stat.st_uid != us {
        return Err(io::Error::other(format!(
            "{} belongs to uid {}, not to you — refusing to use it",
            dir.display(),
            stat.st_uid
        )));
    }
    let mode = stat.st_mode & 0o777;
    if mode != REQUIRED {
        return Err(io::Error::other(format!(
            "{} is mode {:o}, and must be {:o} — refusing to use it",
            dir.display(),
            mode,
            REQUIRED
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests;

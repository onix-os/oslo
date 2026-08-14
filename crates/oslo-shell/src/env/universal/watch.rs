//! Noticing that another shell wrote a universal variable, while this one sits idle.
//!
//! # Why a watch and not a poll
//!
//! The store is re-read before every prompt and again before every command — see
//! [`super::refresh`] — so a value another shell set is picked up the moment you do anything at
//! all. The gap this closes is the one where you do *nothing*: a theme changed in the terminal
//! beside this one should reach a prompt that is merely sitting there.
//!
//! Polling on a timer would mean a `stat` for ever, on every shell, to catch something that happens
//! a few times a day. `inotify` costs one descriptor and wakes only when the file actually moves.
//!
//! # What is watched, and why it is the directory
//!
//! The **parent directory**, not the file. Every write goes through a temporary file and a rename —
//! that is what makes a reader see the old contents or the new and never half of either — and a
//! rename replaces the inode, so a watch on the file itself would be left watching a file nothing
//! writes to again. Watching the directory sees the rename, the creation, and the first write to a
//! store that did not exist when the shell started.

use nix::sys::inotify::{AddWatchFlags, InitFlags, Inotify};

/// Start watching, and register the descriptor so the editor waits on it.
///
/// Quiet about failure on purpose: no `inotify` — an old kernel, a container that forbids it, a
/// home on a filesystem that cannot — costs the *idle* refresh and nothing else. The prompt and
/// pre-command refreshes still happen, so the shell is exactly as correct and slightly less alive.
pub fn start() {
    let Some(path) = super::path() else {
        return;
    };
    let Some(directory) = path.parent() else {
        return;
    };
    // The store's directory may not exist yet — nothing has written a universal variable on this
    // machine. Making it is what lets the watch exist before the first write rather than after it.
    if std::fs::create_dir_all(directory).is_err() {
        return;
    }
    let Ok(inotify) = Inotify::init(InitFlags::IN_NONBLOCK | InitFlags::IN_CLOEXEC) else {
        return;
    };
    let wanted = AddWatchFlags::IN_MOVED_TO
        | AddWatchFlags::IN_CLOSE_WRITE
        | AddWatchFlags::IN_CREATE
        | AddWatchFlags::IN_DELETE;
    if inotify.add_watch(directory, wanted).is_err() {
        return;
    }
    use std::os::fd::{AsFd, AsRawFd};
    oslo_base::background::wake_on(inotify.as_fd().as_raw_fd());
    // **Kept for the life of the process.** The descriptor is registered by number; dropping the
    // `Inotify` would close it, and the kernel would hand that number to the next thing that asked
    // — after which the editor would wake on somebody else's traffic for ever.
    oslo_base::background::keep(inotify);
}

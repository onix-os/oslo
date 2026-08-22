//! Noticing that another shell changed a macro, while this one sits idle.
//!
//! # Why a watch and not a poll
//!
//! The store's stamps are compared before every prompt and again before every command — see
//! `startup::stored::refresh` — so a macro another shell stored is picked up the moment you do
//! anything at all. The gap this closes is the one where you do *nothing*: an `EDITOR` changed in
//! the terminal beside this one should reach a prompt that is merely sitting there.
//!
//! Polling on a timer would mean two `stat`s for ever, on every shell, to catch something that
//! happens a few times a day. `inotify` costs one descriptor and wakes only when a file moves.
//!
//! # This was the universal store's, and it is the reason that store could be deleted
//!
//! `universal` had exactly this watch and macros had none, which made "converges an idle terminal"
//! the one thing it could do that macros could not. Everything else about it — persisting, winning,
//! reaching other shells at their next prompt — a stored variable already did. Moving the watch is
//! what made the two mechanisms one.
//!
//! # What is watched, and why it is the directory
//!
//! The **directory**, not the files. Every write goes through a temporary file and a rename — that
//! is what makes a reader see the old contents or the new and never half of either — and a rename
//! replaces the inode, so a watch on a file would be left watching one nothing writes to again.
//! Watching the directory sees the rename, the creation, and the first write to a store that did
//! not exist when the shell started.

use nix::sys::inotify::{AddWatchFlags, InitFlags, Inotify};

/// Start watching, and register the descriptor so the editor waits on it.
///
/// Quiet about failure on purpose: no `inotify` — an old kernel, a container that forbids it, a
/// home on a filesystem that cannot — costs the *idle* refresh and nothing else. The prompt and
/// pre-command refreshes still happen, so the shell is exactly as correct and slightly less alive.
pub fn start() {
    let Some(directory) = super::directory() else {
        return;
    };
    // The directory may not exist yet — nothing has been stored on this machine. Making it is what
    // lets the watch exist before the first write rather than after it.
    if std::fs::create_dir_all(&directory).is_err() {
        return;
    }
    let Ok(inotify) = Inotify::init(InitFlags::IN_NONBLOCK | InitFlags::IN_CLOEXEC) else {
        return;
    };
    let wanted = AddWatchFlags::IN_MOVED_TO
        | AddWatchFlags::IN_CLOSE_WRITE
        | AddWatchFlags::IN_CREATE
        | AddWatchFlags::IN_DELETE;
    if inotify.add_watch(&directory, wanted).is_err() {
        return;
    }
    use std::os::fd::{AsFd, AsRawFd};
    crate::background::wake_on_with(inotify.as_fd().as_raw_fd(), ours);
    // **Kept for the life of the process.** The descriptor is registered by number; dropping the
    // `Inotify` would close it, and the kernel would hand that number to the next thing that asked
    // — after which the editor would wake on somebody else's traffic for ever.
    crate::background::keep(inotify);
}

/// Empty the watch and answer whether any of it was about the store.
///
/// **The directory holds more than the store.** `macros.sh` and `elsewhere.snapshot` are written by
/// this shell on every mutation, and a `session` file changes for reasons that are nobody else's
/// news — so most of what arrives here is the shell hearing itself. Reacting to those would refresh
/// and rebuild the prompt after every command, for nothing.
///
/// The events have to be read whatever the answer: an `inotify` descriptor that is not emptied
/// stays readable, and the editor would wake on the same events for ever.
fn ours(fd: i32) -> bool {
    // The events are read as bytes rather than through `nix`, which offers no way to read from a
    // descriptor it does not own. The layout is kernel ABI and has not changed since 2.6.13:
    // `wd: i32, mask: u32, cookie: u32, len: u32, name: [u8; len]`, each event aligned to the
    // struct. `len` counts the padding, so the name is NUL-terminated inside it.
    const HEAD: usize = 16;
    let mut interesting = false;
    let mut buffer = [0u8; 4096];
    loop {
        // SAFETY: a borrowed descriptor and a live buffer.
        let n = unsafe { nix::libc::read(fd, buffer.as_mut_ptr().cast(), buffer.len()) };
        if n <= 0 {
            return interesting;
        }
        let mut at = 0usize;
        while at + HEAD <= n as usize {
            let len = u32::from_ne_bytes([
                buffer[at + 12],
                buffer[at + 13],
                buffer[at + 14],
                buffer[at + 15],
            ]) as usize;
            let from = at + HEAD;
            let to = (from + len).min(n as usize);
            if from < to {
                let name = &buffer[from..to];
                let name = &name[..name.iter().position(|b| *b == 0).unwrap_or(name.len())];
                // **The snapshot, which is what a running shell re-reads.** `macros.db` moves on
                // every write including this shell's own, and `macros.sh` is for other shells
                // entirely; the snapshot is the file `stored::refresh` stats.
                if name.starts_with(b"macros.snapshot") || name.starts_with(b"macros.db") {
                    interesting = true;
                }
            }
            at = from + len;
        }
    }
}

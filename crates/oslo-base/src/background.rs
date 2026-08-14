//! Servicing background work from wherever the shell happens to be waiting.
//!
//! # The problem this solves
//!
//! An idle editor is blocked waiting for the terminal. A child exiting, or another shell writing a
//! universal variable, is not a keystroke — so without help the shell learns about it at the *next
//! command boundary*, which for somebody sitting at a prompt means "when you next press Enter". A
//! job that finished ten minutes ago is announced when you run the next thing.
//!
//! # Two ways in, because there are two kinds of event
//!
//! * **A signal**, for a child ending. `SIGCHLD` without `SA_RESTART` makes the blocked `read` fail
//!   with `EINTR` — the route `SIGWINCH` has always taken for a resize.
//! * **A descriptor**, for anything a signal cannot carry: an `inotify` watch on the universal
//!   store, a worker finishing a spawn. Registered here and waited on beside the terminal.
//!
//! Neither carries data. Both mean *something changed, go and look* — [`crate::background::service`] is what looks,
//! and it runs on the shell thread with the locks free.
//!
//! # Why the servicer is installed rather than called
//!
//! The editor is in `oslo-ui`, which sits *below* `oslo-shell` in the crate graph: it cannot name
//! the job table or the environment, and should not. The same inversion as [`crate::hooks`] — the
//! layer that can do the work installs it, the layer that knows *when* calls it.
//!
//! A closure rather than a function pointer, because the work needs state the installer holds: the
//! environment a universal refresh writes into is owned by the REPL.
//!
//! Before installation everything here is a no-op, which is what `sh -c` and every script want.

use std::sync::{Mutex, OnceLock, RwLock};

/// What to run when something in the background may have changed.
static SERVICE: OnceLock<Box<dyn Fn() -> bool + Send + Sync>> = OnceLock::new();

/// Descriptors to wait on beside the terminal. Read-mostly: written once each at startup and read
/// on every editor wait.
static WAKERS: RwLock<Vec<i32>> = RwLock::new(Vec::new());

/// Install the servicer. The first call wins; a later one is ignored rather than panicking, because
/// a second install is a startup-order mistake and not worth killing a shell over.
pub fn install(service: impl Fn() -> bool + Send + Sync + 'static) {
    let _ = SERVICE.set(Box::new(service));
}

/// Do whatever the background needs, if anybody is able to.
///
/// **On the shell thread, between keystrokes, with no editor borrow held** — the same footing as a
/// hook, so it may take locks, reap children, print a notice and run Lua. Never call it from a
/// signal handler: a handler's whole job is to set a flag and let the blocked `read` fail.
///
/// Answers whether anything visible changed, so the caller knows whether a repaint is owed. Most
/// wakes are the shell hearing its own writes to a watched directory, and repainting on every one
/// of those would redraw the prompt after every command.
pub fn service() -> bool {
    match SERVICE.get() {
        Some(service) => service(),
        None => false,
    }
}

/// Whether anything is installed — for a caller deciding whether a wake is worth arranging at all.
pub fn is_installed() -> bool {
    SERVICE.get().is_some()
}

/// Wait on `fd` alongside the terminal, and service the background when it becomes readable.
///
/// The descriptor is **borrowed, not owned**: whoever registers it keeps it open for the life of the
/// shell. A watch that closes would leave a descriptor number here that the kernel has since given
/// to something else, and the editor would wake on somebody else's traffic for ever.
pub fn wake_on(fd: i32) {
    if let Ok(mut wakers) = WAKERS.write()
        && !wakers.contains(&fd)
    {
        wakers.push(fd);
    }
}

/// Every registered descriptor, for the wait.
pub fn wakers() -> Vec<i32> {
    WAKERS
        .read()
        .map(|wakers| wakers.clone())
        .unwrap_or_default()
}

/// Drain a waker that has fired, so the next wait does not fire again on the same event.
///
/// **Whatever it is.** An `inotify` descriptor hands back a struct per event and a pipe hands back
/// bytes; neither is read for its content, because the content is always the same message — go and
/// look. Reading until it would block is the only thing both need.
pub fn drain(fd: i32) {
    let mut scratch = [0u8; 4096];
    loop {
        // SAFETY: a borrowed descriptor and a live buffer. A short read or an error both end this.
        let n = unsafe { nix::libc::read(fd, scratch.as_mut_ptr().cast(), scratch.len()) };
        if n <= 0 {
            return;
        }
        if (n as usize) < scratch.len() {
            return;
        }
    }
}

/// Somewhere for a watch to keep the thing it must not drop.
///
/// A registered descriptor has to outlive the shell's interest in it — see [`wake_on`] — and the
/// owner is usually a local that would otherwise fall out of scope at the end of startup.
static KEPT: Mutex<Vec<Box<dyn std::any::Any + Send>>> = Mutex::new(Vec::new());

/// Keep `owner` alive for the rest of the process.
pub fn keep(owner: impl std::any::Any + Send + 'static) {
    if let Ok(mut kept) = KEPT.lock() {
        kept.push(Box::new(owner));
    }
}

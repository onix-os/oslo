//! Noticing that a background job ended while you were doing nothing.
//!
//! The same shape as [`super::resize`], for the same reason and by the same mechanism: a child
//! exiting is not a keystroke, but the editor is blocked in `read`, so that is where it has to
//! arrive. `SIGCHLD` is installed **without** `SA_RESTART`, the blocked `read` fails with `EINTR`,
//! and the reader asks here whether that was a child.
//!
//! # Why this is not a self-pipe and a poll
//!
//! Because the editor does not have an event loop to add a descriptor to — it has a blocking read
//! that is already interrupted by `SIGWINCH` for exactly this purpose. A second mechanism would be
//! a second thing to get right, and the one that exists is already load-bearing.
//!
//! # What the handler does
//!
//! Sets one flag. Nothing else is safe: reaping calls `waitpid`, printing a job notice allocates,
//! and firing a Lua hook runs an interpreter — all of which are forbidden in a signal handler and
//! all of which happen later, on the shell thread, through [`oslo_base::background::service`].

/// A byte no terminal can send, standing in for "a child ended" inside the key buffer.
///
/// Beside `RESIZE_MARK` and one below it. A marker rather than a side channel so the wake takes its
/// turn in the same queue as the keystrokes around it and cannot overtake one already typed.
pub(super) const CHILD_MARK: u8 = 0xFE;

/// Set by the `SIGCHLD` handler; read and cleared by [`take_child`].
static ENDED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Ask the kernel to interrupt a blocked `read` when a child changes state.
///
/// **Only worth installing for an interactive shell**, and only when something is able to service
/// it: a script reaps at its command boundaries and has no editor to wake.
pub fn watch_for_children() {
    if !oslo_base::background::is_installed() {
        return;
    }
    extern "C" fn on_chld(_: nix::libc::c_int) {
        ENDED.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    // SAFETY: `on_chld` is `extern "C"`, touches one atomic and nothing else, and allocates
    // nothing — the only thing a handler is allowed to do.
    unsafe {
        let action = nix::sys::signal::SigAction::new(
            nix::sys::signal::SigHandler::Handler(on_chld),
            // **No `SA_RESTART`**, which is the whole point: the default has the kernel resume the
            // `read` after the handler returns, and the editor would never learn anything happened.
            nix::sys::signal::SaFlags::empty(),
            nix::sys::signal::SigSet::empty(),
        );
        let _ = nix::sys::signal::sigaction(nix::sys::signal::Signal::SIGCHLD, &action);
    }
}

/// Whether a child has changed state since this was last asked. Clears the flag.
pub(super) fn take_child() -> bool {
    ENDED.swap(false, std::sync::atomic::Ordering::Relaxed)
}

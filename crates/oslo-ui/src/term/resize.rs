//! Noticing that the window changed size.
//!
//! A resize is not a keypress, but it has to arrive through the key reader, because that is where
//! the editor is waiting. A blocked `read` does not return on `SIGWINCH` by itself — the kernel
//! resumes it — so without this a line stays laid out for the old width until you type something.

/// unrecognised sequence is consumed whole and discarded rather than falling through to the text
/// path, which is what makes a mouse wheel do nothing instead of typing.
/// A byte no terminal can send, standing in for "the window changed size" inside the key buffer.
///
/// A marker rather than a second channel, so a resize takes its turn in the same queue as the
/// keystrokes around it and cannot overtake one that was typed first.
pub(super) const RESIZE_MARK: u8 = 0xFF;

/// Set by the `SIGWINCH` handler; read and cleared by [`take_resize`].
static RESIZED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Ask the kernel to interrupt a blocked `read` when the window changes size.
///
/// Installed with **no** `SA_RESTART`, which is the whole point: the default would have the kernel
/// resume the `read` after the handler ran, and the editor would never learn anything happened.
///
/// The handler itself does the only thing a signal handler safely can — set a flag.
pub fn watch_for_resize() {
    extern "C" fn on_winch(_: nix::libc::c_int) {
        RESIZED.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    // SAFETY: `on_winch` is `extern "C"`, touches one atomic and nothing else, and allocates
    // nothing — the only thing a handler is allowed to do.
    unsafe {
        let action = nix::sys::signal::SigAction::new(
            nix::sys::signal::SigHandler::Handler(on_winch),
            nix::sys::signal::SaFlags::empty(),
            nix::sys::signal::SigSet::empty(),
        );
        let _ = nix::sys::signal::sigaction(nix::sys::signal::Signal::SIGWINCH, &action);
    }
}

/// Whether a resize has happened since this was last asked. Clears the flag.
pub(super) fn take_resize() -> bool {
    RESIZED.swap(false, std::sync::atomic::Ordering::Relaxed)
}

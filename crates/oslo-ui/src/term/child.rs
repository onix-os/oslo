//! Noticing that a background job ended while you were doing nothing.
//!
//! A child exiting is not a keystroke, but the editor is waiting on the terminal, so that is where
//! it has to arrive. The handler writes one byte to the wake pipe the editor already waits on
//! beside the terminal — and the wait returns.
//!
//! # Why not `EINTR`, which was tried and measured
//!
//! The obvious route is [`super::resize`]'s: install without `SA_RESTART` and let the blocked
//! `read` fail with `EINTR`. It works, and it costs far more than it looks. That flag is not scoped
//! to the editor — it makes *every* slow syscall in the process interruptible by *every* child
//! exit, and a shell forks constantly. `signal_tests` went from 12 passes in 12 runs to 9, all in
//! `a_loop_whose_body_forks_ends_on_ctrl_c`: a loop that forks, interrupted mid-syscall by its own
//! children finishing.
//!
//! **So `SA_RESTART` stays on and the wake goes down a pipe.** Nothing is interrupted, and the one
//! thing waiting for the news is the one wait that was already watching for it.
//!
//! # What the handler does
//!
//! One relaxed load and one `write` of a single byte. Nothing else is safe: reaping calls `waitpid`,
//! printing a job notice allocates, and firing a Lua hook runs an interpreter — all forbidden in a
//! handler, and all of which happen later on the shell thread through
//! [`oslo_base::background::service`].

/// A byte no terminal can send, standing in for "go and look" inside the key buffer.
///
/// Beside `RESIZE_MARK` and one below it. A marker rather than a side channel so the wake takes its
/// turn in the same queue as the keystrokes around it and cannot overtake one already typed.
pub(super) const CHILD_MARK: u8 = 0xFE;

/// Ask the kernel to tell us when a child changes state, down the wake pipe.
///
/// **Only worth installing for an interactive shell**, and only when something is able to service
/// it: a script reaps at its command boundaries and has no editor to wake.
pub fn watch_for_children() {
    if !oslo_base::background::is_installed() {
        return;
    }
    extern "C" fn on_chld(_: nix::libc::c_int) {
        oslo_base::background::nudge_from_signal();
    }
    // SAFETY: `on_chld` is `extern "C"` and does one relaxed load and one `write` of a single byte,
    // both async-signal-safe. It allocates nothing and takes no lock.
    unsafe {
        let action = nix::sys::signal::SigAction::new(
            nix::sys::signal::SigHandler::Handler(on_chld),
            // **`SA_RESTART`, deliberately.** Without it every child exit interrupts every slow
            // syscall in the process — see the note at the top of this file, which is a measurement
            // rather than a worry.
            nix::sys::signal::SaFlags::SA_RESTART,
            nix::sys::signal::SigSet::empty(),
        );
        let _ = nix::sys::signal::sigaction(nix::sys::signal::Signal::SIGCHLD, &action);
    }
}

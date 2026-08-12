//! Where "later" happens.
//!
//! One line in the loop, so that `repl.rs` says what it does rather than how. The rule and the
//! reasoning are in [`crate::lua::api::timer`]: a timer fires between commands, never during one.

/// Run whatever a config asked to happen by now.
///
/// **The check before the work.** This is called twice per command — at the top of the loop and
/// after the command finishes — and a session with no timers is the overwhelming case, so it must
/// cost one relaxed read of a list length rather than an `Instant::now` and a scan.
pub(super) fn fire() {
    if crate::lua::api::timer::any() {
        crate::lua::api::timer::fire_due();
    }
}

/// Everything that waited for the command to be over.
///
/// Two things queue on the same condition — the shell holding nothing — so they are named together
/// rather than as two consecutive lines in the loop: a hook a builtin deferred because it could not
/// act from where it fired, and a timer that came due while something was running.
pub(super) fn after_command() {
    crate::lua::engine::run_deferred_hooks();
    fire();
}

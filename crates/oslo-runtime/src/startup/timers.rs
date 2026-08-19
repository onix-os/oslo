//! Where "later" happens.
//!
//! One line in the loop, so that `repl.rs` says what it does rather than how. The rule and the
//! reasoning are in [`crate::lua::api::timer`]: a timer fires between commands, never during one.

/// Run whatever a config asked to happen by now.
///
/// **The check before the work.** This is called twice per command — at the top of the loop and
/// after the command finishes — and a session with no timers is the overwhelming case, so it must
/// cost one relaxed read of a list length rather than an `Instant::now` and a scan.
/// Answers whether any handler actually ran.
///
/// **Which is what tells an idle prompt it is out of date.** A prompt is expanded when it is built,
/// not on every repaint, so a handler that changed what `oslo.prompt.left` reads leaves a correct
/// value on screen beside a stale prompt — and the servicer was rebuilding only for a changed
/// universal variable, so a timer's effect appeared whenever something else happened to invalidate
/// and not otherwise.
pub(super) fn fire() -> bool {
    let mut ran = false;
    if crate::lua::api::timer::any() {
        ran |= crate::lua::api::timer::fire_due();
    }
    // A background process that finished while something else was running. The same safe point, for
    // the same reason: this is where the shell holds nothing and can call Lua.
    ran | crate::lua::api::spawn::deliver_if_any()
}

/// Everything that waited for the command to be over.
///
/// Two things queue on the same condition — the shell holding nothing — so they are named together
/// rather than as two consecutive lines in the loop: a hook a builtin deferred because it could not
/// act from where it fired, and a timer that came due while something was running.
pub(super) fn after_command() {
    crate::lua::engine::run_deferred_hooks();
    // The prompt is rebuilt after every command anyway, so what ran here needs no announcing.
    let _ = fire();
}

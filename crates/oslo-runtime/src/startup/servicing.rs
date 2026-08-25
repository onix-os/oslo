//! What an idle prompt does when the background moves.
//!
//! Split from [`super::repl`] when that file crossed the 600-line limit, along the seam it already
//! had: everything else there is the loop that reads a line and runs it, and this is the one part
//! that runs when *nothing* is being read. It is installed once and then called by
//! [`oslo_base::background`] from the editor's input wait.
//!
//! # The safe point
//!
//! Every step here runs on the shell's own thread with no editor borrow held, which is what lets
//! them take locks, reap children, print and run Lua. That is also why a peer's `cd` is made here
//! rather than where it is asked for — see [`super::follow`].

use super::timers;
use oslo_shell::env::Environment;
use std::sync::{Arc, Mutex};

/// Install the servicer. Called once, before anything can finish on a thread.
pub fn install(env: &Arc<Mutex<Environment>>, macros: &Arc<Mutex<super::stored::Held>>) {
    let serviced = Arc::clone(env);
    let held_for_service = Arc::clone(macros);
    oslo_base::background::install_deadline(crate::lua::api::timer::next_due_in_ms);
    // Before anything can finish on a thread, so the descriptor is in the set the first wait polls.
    oslo_base::background::arm();
    oslo_base::background::install(move || {
        oslo_shell::exec::job::reap_background_jobs();
        // A timer that came due, and anything `oslo.spawn` finished on a thread. The same safe
        // point they already use at a command boundary — the shell holds nothing and Lua may run.
        let fired = timers::fire();
        // A macro another terminal stored — an alias, an abbreviation, a variable. Two `stat`s
        // decide whether there is anything to read, so a wake for some other reason costs that and
        // nothing more.
        let changed = match held_for_service.lock() {
            Ok(mut held) => super::stored::refresh(&serviced, &mut held),
            Err(_) => false,
        };
        // A directory a connected peer asked for — see `lua::api::live::queued` for why it is
        // asked for rather than done. Here is the safe point: the shell's own thread, holding
        // nothing, which is what makes a process-wide `chdir` safe to make.
        let moved = super::follow::follow(&serviced);
        // **The lock is gone by here, so anything the apply announced can run.** A hook fired while
        // the shell's state is held is queued rather than called — it could look but not touch —
        // and the queue is drained when the borrow that held it ends. This servicer takes the lock
        // directly rather than through that borrow, so without this the `on-variable-change` for a
        // value another terminal just set waited for the next command, which is the whole thing an
        // idle wake exists to avoid.
        crate::lua::engine::run_deferred_hooks();
        // **A changed variable is invisible until the prompt is rebuilt.** `PS1` is expanded when
        // the prompt is rendered, not on every repaint — so a theme another terminal just set would
        // otherwise sit in the environment, correct and unseen, until the next command. This is the
        // same door an asynchronous prompt already comes through.
        //
        // **A handler that ran is the same kind of news.** `oslo.after(1200, function() mood =
        // "after" end)` changes what the prompt function returns, and without this the prompt was
        // rebuilt only when a *universal variable* had also changed — so the new prompt appeared
        // whenever something else happened to invalidate, and not otherwise.
        if changed || fired || moved {
            oslo_ui::prompt::invalidate();
        }
        changed || fired || moved
    });
}

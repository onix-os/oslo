//! Moving because a connected peer said so.
//!
//! The other half of [`crate::lua::api::live::queued`], which explains why the move is asked for on
//! one thread and made on another. This is the making of it, and it runs from the servicer
//! [`oslo_base::background::install`] was given — the shell's own thread, between keystrokes,
//! holding nothing.
//!
//! # It goes through `cd`, and that is the whole design
//!
//! Not `set_current_dir`. A directory change is more than the kernel's idea of where the process
//! is: `$PWD` and `$OLDPWD` have to move with it, the ring of directories you have been in has to
//! record it, and `post-change-dir` has to fire. `builtin_cd` does all of that already, for `cd`,
//! for `pushd`, for a jump bound to a key — and a second route that did only the first part would
//! leave a shell whose `$PWD` disagreed with its prompt.
//!
//! # What it does *not* do
//!
//! Run the directory's `.env.lua`. That happens at the top of the prompt loop, against the
//! directory last *settled* — see the comment there — and it is a prompt-boundary concern on
//! purpose: arriving in the middle of a half-typed line would change `$PATH` under a completion
//! already in flight. A peer's move is noticed by that check like any other, on the next prompt.

use oslo_shell::env::Environment;
use std::sync::{Arc, Mutex};

/// Make the move a peer asked for, if one is waiting. Answers whether anything changed.
///
/// **The lock is taken and released here**, rather than being handed in: the servicer's other
/// steps want it free, and a `cd` that could not take it is a `cd` that has to be dropped rather
/// than deferred — the shell is running something, and a directory change landing part-way through
/// a command is the thing this whole path exists to prevent.
pub fn follow(env: &Arc<Mutex<Environment>>) -> bool {
    let Some(dir) = crate::lua::api::live::queued::take() else {
        return false;
    };
    let Ok(mut held) = env.try_lock() else {
        // **Held means a command is running, and one of those is a file browser.** A browser opened
        // in a float is the case this whole path exists for: it moves the shell as you walk, and
        // the prompt beside it should say so — but it is `builtin_nav` that is running, holding the
        // shell state for as long as the browser is up, so the full move cannot be made yet.
        //
        // So the *kernel's* idea of where we are moves now, which is what a prompt reads, and the
        // request stays in the slot for the next safe point to finish properly: `$PWD`, `$OLDPWD`,
        // the directory ring and `post-change-dir` are all still owed and are all still coming.
        //
        // Safe here and not from the server thread, which is the distinction `queued` draws: this
        // is the shell's own thread, and the only other party is a child process with a working
        // directory of its own.
        let moved = std::env::set_current_dir(&dir).is_ok();
        crate::lua::api::live::queued::ask(dir);
        return moved;
    };
    let words = ["cd".to_string(), dir.to_string_lossy().into_owned()];
    match oslo_shell::env::builtins::builtin_cd(&mut held, &words) {
        Ok(0) => true,
        // `cd` has already said what was wrong, on stderr, in its own words. Answering `false` only
        // means the prompt does not need rebuilding.
        _ => false,
    }
}

#[cfg(test)]
#[path = "follow/tests.rs"]
mod tests;

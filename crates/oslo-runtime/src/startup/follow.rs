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
        oslo_base::messages::say(
            oslo_base::messages::Level::Note,
            "live",
            format!("cd {}: the shell is busy", dir.display()),
        );
        return false;
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

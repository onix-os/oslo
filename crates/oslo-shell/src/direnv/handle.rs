//! The shell's one handle on its directory environment.
//!
//! Split from the lifecycle next door because it answers a different question: that module decides
//! *what should happen* when you arrive somewhere, and this one decides *who can ask*. Two callers
//! need it and neither owns the other — the read loop drives the lifecycle on every `cd`, and the
//! `direnv` builtin reads and writes the allow list — so it is a process global rather than
//! something threaded through every builtin's signature for the sake of one of them.

use super::Direnv;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

/// What this shell has, if anything.
static STATE: Mutex<Option<Direnv>> = Mutex::new(None);

/// Hand the shell its directory-environment state. Called once at startup.
pub fn install(direnv: Direnv) {
    if let Ok(mut slot) = STATE.lock() {
        *slot = Some(direnv);
    }
}

/// Do something with the state, if this shell has any.
///
/// Answers `None` in a script: a non-interactive shell has no directory environment at all, for the
/// same reason it has no tracking store. A script's environment comes from whoever ran it, and a
/// file in the working directory silently changing it would make scripts depend on where they are
/// invoked from.
pub fn with<T>(f: impl FnOnce(&mut Direnv) -> T) -> Option<T> {
    let mut slot = STATE.lock().ok()?;
    slot.as_mut().map(f)
}

/// Whether this shell has a directory environment at all.
pub fn installed() -> bool {
    STATE.lock().map(|slot| slot.is_some()).unwrap_or(false)
}

/// Set when a decision was made that the loaded state does not reflect yet.
///
/// The `direnv` builtin cannot do the reloading itself: running `.env.lua` needs the Lua engine,
/// which belongs to the read loop and not to a builtin's arguments. So it leaves a request behind
/// and the loop carries it out at the end of the command — the same shape `history -c` already uses
/// for the same reason.
static RELOAD: AtomicBool = AtomicBool::new(false);

/// Ask the read loop to bring the environment into line before the next prompt.
pub fn request_reload() {
    RELOAD.store(true, Ordering::Relaxed);
}

/// Take the pending request, if there is one.
pub fn take_reload_request() -> bool {
    RELOAD.swap(false, Ordering::Relaxed)
}

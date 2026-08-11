//! Arriving in a directory, for a build that cares what is in it.
//!
//! **The whole of the `direnv` feature wall on the REPL's side.** The loop calls these three
//! whether or not the feature is on; without it they do nothing and nothing in `repl.rs` says so.
//! The alternative — a `#[cfg]` at each of the four call sites — put the condition in the middle of
//! the loop's own logic three times over, where the next person editing the loop has to reason
//! about it to change something unrelated.

#[cfg(feature = "direnv")]
use crate::lua::LuaEngine;
#[cfg(feature = "direnv")]
use oslo_shell::Environment;
#[cfg(feature = "direnv")]
use std::path::Path;
#[cfg(feature = "direnv")]
use std::sync::{Arc, Mutex};

/// Begin the session's directory-environment bookkeeping.
pub(super) fn start() {
    #[cfg(feature = "direnv")]
    super::environments::start();
}

/// Load whatever `path` asks for, and unload what the last directory did.
#[cfg(feature = "direnv")]
pub(super) fn arrive(env: &Arc<Mutex<Environment>>, lua: &LuaEngine, path: &Path) {
    super::environments::arrive(env, lua, path);
}

/// Without the feature there is nothing to arrive into, and the arguments are the loop's own.
#[cfg(not(feature = "direnv"))]
pub(super) fn arrive<E, L, P>(_env: &E, _lua: &L, _path: P) {}

/// Whether `direnv allow` asked for a reload it could not do itself.
///
/// A builtin cannot run an `.env.lua`: that needs the Lua engine, which lives in the loop. So the
/// builtin leaves a request and the loop carries it out.
pub(super) fn reload_requested() -> bool {
    #[cfg(feature = "direnv")]
    {
        oslo_shell::direnv::take_reload_request()
    }
    #[cfg(not(feature = "direnv"))]
    false
}

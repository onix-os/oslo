//! **The whole of the `plugin` feature wall on the REPL's side.**
//!
//! The loop calls both of these whether or not the feature is on; without it they do nothing and
//! `repl.rs` says nothing about it. `startup::arrival` does the same for `direnv`, for the same
//! reason: a `#[cfg]` in the middle of the loop is a condition the next person editing the loop has
//! to reason about to change something unrelated.

/// Read the index, so the loop knows which words are worth loading a plugin for.
#[cfg(feature = "plugin")]
pub(super) fn start(env: &std::sync::Arc<std::sync::Mutex<oslo_shell::Environment>>) {
    // A name the config already answers to is the config's. Asked here, once, while the lock is
    // free — the loader itself must never take it.
    let Ok(guard) = env.lock() else {
        return;
    };
    crate::plugin::start(|name| guard.is_builtin(name));
}

#[cfg(not(feature = "plugin"))]
pub(super) fn start<E>(_env: &E) {}

/// Load any plugin this line mentions, before the line runs.
///
/// **Called with the shell's state free**, which is the whole reason it is here rather than inside a
/// builtin: registering a builtin needs that state, so a plugin loaded from inside one could never
/// register what it was loaded to provide.
#[cfg(feature = "plugin")]
pub(super) fn before(line: &str) {
    crate::plugin::ensure_loaded(line);
}

#[cfg(not(feature = "plugin"))]
pub(super) fn before(_line: &str) {}

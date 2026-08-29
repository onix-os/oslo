//! **The whole of the `plugin` feature wall on the REPL's side.**
//!
//! The loop calls this whether or not the feature is on; without it it does nothing and `repl.rs`
//! says nothing about it. `startup::arrival` does the same for `direnv`, for the same reason: a
//! `#[cfg]` in the middle of the loop is a condition the next person editing the loop has to reason
//! about to change something unrelated.

/// Run every plugin on the runtimepath.
///
/// **Called with the shell's state free**, which is the whole reason it is here rather than inside a
/// builtin: registering a builtin needs that state, so a plugin loaded from inside one could never
/// register what it was loaded to provide.
///
/// After the config, deliberately: `init.lua` runs first, so a `oslo.plugin.secrets` grant in it is
/// in place before the plugin it names loads.
#[cfg(feature = "plugin")]
pub(super) fn start() {
    crate::plugin::load_all();
}

#[cfg(not(feature = "plugin"))]
pub(super) fn start() {}

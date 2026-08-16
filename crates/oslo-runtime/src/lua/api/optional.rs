//! The namespaces a build may or may not have.
//!
//! Every one is behind its cargo feature, which is why they are gathered here rather than sitting
//! among the unconditional ones: a reader asking "what does `oslo-minimal` not have?" gets one
//! file, and a config asks the same question the way `docs/features/runtime-features.md` says —
//! `if oslo.nix then … end`.

use oslo_lua::Interp;
use oslo_base::value::Table;
// Every use of it is behind a feature, so a default build has none.
#[cfg(any(
    feature = "direnv",
    feature = "nix",
    feature = "secrets",
    feature = "plugin"
))]
use oslo_base::value::Value;
use oslo_shell::env::Environment;
use std::sync::{Arc, Mutex};

/// The namespaces a build may or may not have, each behind its cargo feature — so a config asks
/// whether one is there, `if oslo.nix then`, as `docs/features/runtime-features.md` says.
#[allow(unused_variables)]
pub fn install(oslo: &mut Table, interp: &Interp, env: &Arc<Mutex<Environment>>) {
    #[cfg(feature = "direnv")]
    oslo.set(Value::str("direnv"), super::direnv::build(env));
    #[cfg(feature = "nix")]
    oslo.set(Value::str("nix"), super::nix::build(interp));
    #[cfg(feature = "secrets")]
    oslo.set(Value::str("secret"), super::secret::build());
    #[cfg(feature = "plugin")]
    oslo.set(Value::str("plugin"), crate::plugin::health::build());
}

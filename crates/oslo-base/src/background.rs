//! Servicing background work from wherever the shell happens to be waiting.
//!
//! # The problem this solves
//!
//! An idle editor is blocked in `read` on the terminal. A child exiting is not a keystroke, so
//! without help the shell learns about it at the *next command boundary* — which for somebody
//! sitting at a prompt means "when you next press Enter". A job that finished ten minutes ago is
//! announced when you run the next thing, and until then `jobs` at that prompt is a lie.
//!
//! # Why a function pointer, and not a call
//!
//! The editor is in `oslo-ui`, which sits *below* `oslo-shell` in the crate graph — it cannot name
//! the job table, and should not. The same inversion as [`crate::hooks`]: the layer that can do the
//! work installs a pointer once at startup, and the layer that knows *when* calls it.
//!
//! Before installation this is a no-op, which is what `sh -c` and every script want: they have no
//! editor, they reap at command boundaries, and nothing here ever runs.
//!
//! # What is safe to do from it
//!
//! It is called on the shell thread, between keystrokes, with no borrow of the editor held — the
//! same footing as a hook. So it may take locks, reap children, print a job notice and fire Lua.
//! It is **not** a signal handler and must never be called from one: the handler's whole job is to
//! set a flag and let the blocked `read` fail with `EINTR`.

use std::sync::OnceLock;

/// What to run when something in the background may have changed.
static SERVICE: OnceLock<fn()> = OnceLock::new();

/// Install the servicer. The first call wins; later ones are ignored rather than panicking, because
/// a second install is a startup-order mistake and not worth killing a shell over.
pub fn install(service: fn()) {
    let _ = SERVICE.set(service);
}

/// Do whatever the background needs, if anybody is able to.
pub fn service() {
    if let Some(service) = SERVICE.get() {
        service();
    }
}

/// Whether anything is installed — for a caller deciding whether a wake is worth arranging at all.
pub fn is_installed() -> bool {
    SERVICE.get().is_some()
}

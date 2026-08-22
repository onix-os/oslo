//! Telling a config that a variable changed.
//!
//! # Why the payload is five fields and not one
//!
//! "`PAGER` changed" is not enough to act on. A status line wants to know whether the change came
//! from this shell or from another window; a plugin syncing state wants to know whether the name
//! was set or taken away; something managing `PATH` wants to know whether it is exported. Answering
//! those from the hook means reading the variable back, by which time a later change may already
//! have overwritten it.
//!
//! # Where it fires
//!
//! The four places a *user* changes a variable: an assignment, `export`, `unset`, and the macro
//! store, when it is re-read because another process wrote it.
//! Deliberately not `Environment::set_var`, which is also how the shell maintains `PWD`, `?`, `_`
//! and the rest: firing there would bury a config's handler in the shell's own bookkeeping.

use oslo_base::hooks::{at, fire_at_here, watched};

/// What happened to the name.
pub enum Change {
    /// It has a value now. Carries whether that value is exported.
    Set { exported: bool },
    /// It is gone.
    Erased,
}

/// Where the name lives.
pub enum Scope {
    /// An ordinary shell variable, exported or not.
    Shell,
    /// The macro store, which follows you between terminals.
    Stored,
}

/// Which shell did it.
pub enum Source {
    /// This one.
    Local,
    /// Another one, and this shell found out by re-reading the store.
    Remote,
}

/// Tell `on-variable-change` about one name.
///
/// **The `watched` check is not an optimisation here, it is the reason this can sit on the
/// assignment path at all.** Every `x=1` in every script reaches this function; with nothing
/// attached it costs one relaxed load and returns.
pub fn announce(name: &str, change: Change, scope: Scope, source: Source) {
    if !watched(at::VARIABLE_CHANGE) {
        return;
    }
    let (action, exported) = match change {
        Change::Set { exported } => ("set", exported),
        Change::Erased => ("erase", false),
    };
    fire_at_here(
        at::VARIABLE_CHANGE,
        &[
            ("name", name),
            ("action", action),
            (
                "scope",
                match scope {
                    Scope::Shell => "shell",
                    Scope::Stored => "stored",
                },
            ),
            (
                "source",
                match source {
                    Source::Local => "local",
                    Source::Remote => "remote",
                },
            ),
            ("exported", if exported { "true" } else { "false" }),
        ],
    );
}

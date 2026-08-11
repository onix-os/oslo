//! Whether the command now running must leave no trace of itself.
//!
//! Everything the shell writes down about a command is decided *around* the command — the history
//! line before it, the tracking row after it. One thing is not: `set -x` prints each expanded
//! argument from inside execution, where the loop that made the decision is several frames up and
//! has no way to say so.
//!
//! This is that channel, and it is deliberately the narrowest one that works: a flag that is true
//! for exactly as long as a vetoed command is executing.
//!
//! # Thread-local, and why that is enough
//!
//! Execution and the read loop are the same thread. A background job forked from here is a
//! different *process* and inherits nothing, which is correct: `set -x` in a subshell is that
//! shell's business.

use std::cell::Cell;

thread_local! {
    static QUIET: Cell<bool> = const { Cell::new(false) };
}

/// Is the command being executed one that must not be traced?
pub fn active() -> bool {
    QUIET.with(Cell::get)
}

/// Make it so until the guard is dropped.
///
/// **A guard rather than a pair of calls**, because the alternative is a command that returns early
/// — a failed redirection, a `Ctrl-C`, an error in a hook — leaving the flag set for the rest of the
/// session and silently disabling `set -x` from then on.
#[must_use = "the flag is cleared when this is dropped, so dropping it immediately does nothing"]
pub struct Quiet {
    /// What it was before, so nesting restores rather than clears.
    was: bool,
}

impl Quiet {
    pub fn enter() -> Quiet {
        let was = QUIET.replace(true);
        Quiet { was }
    }
}

impl Drop for Quiet {
    fn drop(&mut self) {
        QUIET.set(self.was);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_flag_is_only_set_while_the_guard_lives() {
        assert!(!active());
        {
            let _quiet = Quiet::enter();
            assert!(active());
        }
        assert!(!active(), "the guard must clear it on the way out");
    }

    #[test]
    fn nesting_restores_rather_than_clears() {
        let _outer = Quiet::enter();
        {
            let _inner = Quiet::enter();
            assert!(active());
        }
        assert!(active(), "the inner guard must not end the outer one");
    }
}

//! Command names hidden from a bare-word `$PATH` search, decided per directory.
//!
//! ```lua
//! oslo.command.when("direnv", function(dir)
//!   return oslo.fs.find_up(".envrc", dir) ~= nil
//! end)
//! ```
//!
//! The sibling of [`crate::feature`], for the other half of what a word can name. A feature turns
//! off a **builtin**, from a fixed table, because a builtin the shell does not gate is a typo. This
//! turns off a **program on `$PATH`**, and those are an open set: no table could list them, and the
//! whole point is to name one the shell has never heard of.
//!
//! # A mask, and `$PATH` is not touched
//!
//! Hiding is oslo's own opinion about a word, applied where the word is resolved. `$PATH` in the
//! environment is left exactly as it was, so every program the shell launches still sees the real
//! one — including the hidden program, when something reaches it by an absolute path or a child
//! shell looks it up for itself.
//!
//! Rewriting `$PATH` per directory would do the opposite: leak the shell's opinion into every
//! process started from that directory, into anything they start in turn, and into files like
//! `.envrc` that are entitled to a `$PATH` the user actually set. It would also have to be put
//! back on the way out, which is the state this design exists to not have.
//!
//! A word with a slash in it is a path and not a search — POSIX 2.9.1.1 — so it is never hidden.
//! `./direnv` and `/usr/bin/direnv` run whatever is there, which is the same escape hatch
//! `command` gives for a shadowed builtin.
//!
//! # Recomputed, never restored
//!
//! [`apply`] takes the whole set for the directory the shell is now in, so leaving a directory is
//! the same operation as arriving in one: ask again, get the other answer. There is no
//! "unhide on the way out" to forget, which is the same property [`crate::feature`]'s mask has and
//! for the same reason.
//!
//! # Cost
//!
//! One relaxed atomic load per resolved command when nothing is hidden, which is the case for every
//! configuration that never calls `when`. A set that is genuinely non-empty costs a read lock and a
//! hash on top, on the path that is about to `fork` anyway.

use std::collections::BTreeSet;
use std::sync::RwLock;
use std::sync::atomic::{AtomicBool, Ordering};

/// Whether anything at all is hidden, so the common case never takes the lock.
static ANY: AtomicBool = AtomicBool::new(false);

/// The hidden names. Ordered so [`listing`] is stable and a diagnostic reads the same twice.
static HIDDEN: RwLock<BTreeSet<String>> = RwLock::new(BTreeSet::new());

/// Whether `name` is currently hidden from a bare-word search.
pub fn hidden(name: &str) -> bool {
    if !ANY.load(Ordering::Relaxed) {
        return false;
    }
    HIDDEN.read().is_ok_and(|names| names.contains(name.trim()))
}

/// Replace the hidden set outright, answering whether it changed.
///
/// **The whole set, not one name.** A caller deciding what a directory hides has just computed the
/// answer for every name it knows about; handing them over one at a time would need a matching
/// "and unhide the rest", which is the bookkeeping this avoids.
///
/// The `bool` is what a caller invalidates a cache on: the command index is built from `$PATH` with
/// these names taken out, so it is stale exactly when this returns `true` — and rebuilding it costs
/// a `$PATH` walk, which is not something to do on every prompt for a set that did not move.
pub fn apply(names: BTreeSet<String>) -> bool {
    let Ok(mut held) = HIDDEN.write() else {
        return false;
    };
    if *held == names {
        return false;
    }
    ANY.store(!names.is_empty(), Ordering::Relaxed);
    *held = names;
    true
}

/// Every hidden name, for `oslo.command.hidden()` and for a diagnostic.
pub fn listing() -> Vec<String> {
    if !ANY.load(Ordering::Relaxed) {
        return Vec::new();
    }
    HIDDEN
        .read()
        .map(|names| names.iter().cloned().collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(names: &[&str]) -> bool {
        apply(names.iter().map(|n| (*n).to_string()).collect())
    }

    /// **One test, because the state is process-global.** Two `#[test]`s would run on two threads
    /// against the same set and take turns failing, which is a worse bug than the one they were
    /// written to catch — the same reason [`crate::feature`] keeps its own to a single case.
    #[test]
    fn the_mask_hides_replaces_and_reports_change() {
        set(&[]);
        assert!(!hidden("direnv"), "nothing is hidden until something is");

        assert!(set(&["direnv"]), "the first set is a change");
        assert!(hidden("direnv"));
        assert!(!hidden("ls"), "only the named ones");
        assert!(hidden("  direnv  "), "asked about with whitespace");

        assert!(!set(&["direnv"]), "the same set again is not a change");
        assert!(set(&["other"]), "a different set is");

        // Replaced outright: leaving a directory is arriving with a different answer, and there is
        // nothing to put back.
        assert!(!hidden("direnv"));
        assert!(hidden("other"));

        assert!(set(&[]), "emptying is a change");
        assert!(!hidden("other"));
        assert!(listing().is_empty());
    }
}

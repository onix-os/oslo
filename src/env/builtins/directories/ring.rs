//! The directories you have actually been in, and how to go back.
//!
//! ```text
//! cd -3          three back
//! dirh           the ring, newest last
//! ```
//!
//! **`cd -` is a one-deep toggle**, and useless the moment you are three wrong turns from where you
//! meant to be. This records every move and lets `cd -N` reach any of them, which is among the
//! highest-frequency things anybody does at a shell all day. `dirh` is how you see the numbers
//! `cd -N` takes; without it `cd -3` is a guess.
//!
//! Deliberately *not* `pushd`/`popd`. Those are explicit — you say when to remember and when to
//! forget, and a script relies on that. This is automatic and interactive, and the two must not
//! share a store or `popd` in a script would start finding directories nobody pushed.
//!
//! # What used to be here
//!
//! `prevd`/`nextd` walked this ring backwards and forwards with a cursor. They are gone: `cd -` and
//! `cd -N` already reach every entry, and the cursor was the only thing that made this file
//! complicated. Their `walk_to` also called `set_current_dir` and assigned `$PWD` by hand while
//! never touching `$OLDPWD`, so walking with `prevd` silently desynchronised `cd -` — it would send
//! you back to wherever the last real `cd` had come from, which by then was somewhere you had left
//! several moves ago. That is a bug being deleted rather than a feature.

use crate::env::Environment;
use crate::error::Result;
use std::sync::{Mutex, OnceLock};

/// How many directories are worth remembering.
///
/// Enough to cover a session's wandering, small enough that `dirh` is still readable at a glance.
const DEPTH: usize = 32;

#[derive(Default)]
struct Ring {
    /// Oldest first; the last entry is where you are now.
    visited: Vec<String>,
}

fn ring() -> &'static Mutex<Ring> {
    static RING: OnceLock<Mutex<Ring>> = OnceLock::new();
    RING.get_or_init(|| Mutex::new(Ring::default()))
}

/// Record a directory the shell has moved to.
pub fn record(path: &str) {
    let Ok(mut ring) = ring().lock() else {
        return;
    };
    if ring.visited.last().is_some_and(|last| last == path) {
        return;
    }
    ring.visited.push(path.to_string());
    if ring.visited.len() > DEPTH {
        ring.visited.remove(0);
    }
}

/// The ring, oldest first.
pub fn history() -> Vec<String> {
    ring().lock().map(|r| r.visited.clone()).unwrap_or_default()
}

/// The directory `n` steps back, without moving.
///
/// `nth_back(0)` is where you are standing, so `cd -1` and `cd -` name the same directory in a
/// shell whose ring was seeded with the directory it started in.
pub fn nth_back(n: usize) -> Option<String> {
    let ring = ring().lock().ok()?;
    let at = ring.visited.len().checked_sub(1)?;
    ring.visited.get(at.checked_sub(n)?).cloned()
}

pub fn builtin_dirh(_env: &mut Environment, _args: &[String]) -> Result<i32> {
    let visited = history();
    let width = visited.len().to_string().len();
    for (i, path) in visited.iter().rev().enumerate() {
        // Numbered by how far back each one is, because that is the number `cd -N` takes.
        println!("{i:>width$}  {path}");
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh() {
        if let Ok(mut ring) = ring().lock() {
            *ring = Ring::default();
        }
    }

    /// One process-wide ring, so every test that touches it takes this first.
    static SERIAL: Mutex<()> = Mutex::new(());

    /// `cd -N` counts back from where you are.
    #[test]
    fn nth_back_counts_from_here() {
        let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        fresh();
        for path in ["/a", "/b", "/c"] {
            record(path);
        }
        assert_eq!(nth_back(0).as_deref(), Some("/c"));
        assert_eq!(nth_back(1).as_deref(), Some("/b"));
        assert_eq!(nth_back(2).as_deref(), Some("/a"));
        assert_eq!(nth_back(3), None);
    }

    /// Going where you already are is not a move worth recording.
    #[test]
    fn the_same_directory_twice_is_one_entry() {
        let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        fresh();
        record("/a");
        record("/a");
        assert_eq!(history(), ["/a"]);
    }

    /// The ring only ever grows forwards now. Nothing abandons an entry that was recorded before
    /// it, which is what the walking cursor used to do the moment you moved somewhere new.
    #[test]
    fn moving_on_never_discards_where_you_have_been() {
        let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        fresh();
        for path in ["/a", "/b", "/c"] {
            record(path);
        }
        let back = nth_back(2);
        record("/d");
        assert_eq!(history(), ["/a", "/b", "/c", "/d"]);
        assert_eq!(back.as_deref(), Some("/a"));
        assert_eq!(nth_back(3).as_deref(), Some("/a"), "and still reachable");
    }

    /// The oldest entries fall off rather than the ring growing for the life of the shell.
    #[test]
    fn the_ring_is_bounded() {
        let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        fresh();
        for i in 0..DEPTH + 5 {
            record(&format!("/d{i}"));
        }
        let visited = history();
        assert_eq!(visited.len(), DEPTH);
        assert_eq!(visited.first().map(String::as_str), Some("/d5"));
        assert_eq!(
            nth_back(0).as_deref(),
            Some(&format!("/d{}", DEPTH + 4)[..])
        );
    }
}

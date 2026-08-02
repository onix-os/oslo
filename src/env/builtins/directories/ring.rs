//! The directories you have actually been in, and how to go back.
//!
//! ```text
//! prevd          back one
//! nextd          forward again
//! cd -3          three back
//! dirh           the ring, newest last
//! ```
//!
//! **`cd -` is a one-deep toggle**, and useless the moment you are three wrong turns from where you
//! meant to be. This records every `cd` and lets you walk it, which is among the highest-frequency
//! things anybody does at a shell all day.
//!
//! Deliberately *not* `pushd`/`popd`. Those are explicit — you say when to remember and when to
//! forget, and a script relies on that. This is automatic and interactive, and the two must not
//! share a store or `popd` in a script would start finding directories nobody pushed.

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
    /// How far back `prevd` has walked, so `nextd` can walk forward again.
    back: usize,
}

fn ring() -> &'static Mutex<Ring> {
    static RING: OnceLock<Mutex<Ring>> = OnceLock::new();
    RING.get_or_init(|| Mutex::new(Ring::default()))
}

/// Record a directory the shell has moved to.
///
/// Walking back and then moving somewhere new abandons the forward history, exactly as a browser
/// does — the alternative is a "forward" that goes somewhere you never chose.
pub fn record(path: &str) {
    let Ok(mut ring) = ring().lock() else {
        return;
    };
    if ring.back > 0 {
        let keep = ring.visited.len() - ring.back;
        ring.visited.truncate(keep);
        ring.back = 0;
    }
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
pub fn nth_back(n: usize) -> Option<String> {
    let ring = ring().lock().ok()?;
    let at = ring.visited.len().checked_sub(1 + ring.back)?;
    ring.visited.get(at.checked_sub(n)?).cloned()
}

/// Walk one step back, answering where to go.
fn step_back() -> Option<String> {
    let mut ring = ring().lock().ok()?;
    let position = ring.visited.len().checked_sub(1 + ring.back)?;
    let target = ring.visited.get(position.checked_sub(1)?)?.clone();
    ring.back += 1;
    Some(target)
}

/// Walk one step forward, answering where to go.
fn step_forward() -> Option<String> {
    let mut ring = ring().lock().ok()?;
    if ring.back == 0 {
        return None;
    }
    ring.back -= 1;
    let position = ring.visited.len().checked_sub(1 + ring.back)?;
    ring.visited.get(position).cloned()
}

/// Change directory without disturbing the ring's position — a walk is not a new destination.
fn walk_to(env: &mut Environment, path: &str) -> Result<i32> {
    match std::env::set_current_dir(path) {
        Ok(()) => {
            env.set_var("PWD", path, true);
            println!("{path}");
            Ok(0)
        }
        Err(e) => {
            eprintln!("oslo: {path}: {e}");
            Ok(1)
        }
    }
}

pub fn builtin_prevd(env: &mut Environment, _args: &[String]) -> Result<i32> {
    match step_back() {
        Some(path) => walk_to(env, &path),
        None => {
            eprintln!("oslo: prevd: nowhere further back");
            Ok(1)
        }
    }
}

pub fn builtin_nextd(env: &mut Environment, _args: &[String]) -> Result<i32> {
    match step_forward() {
        Some(path) => walk_to(env, &path),
        None => {
            eprintln!("oslo: nextd: nowhere further forward");
            Ok(1)
        }
    }
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

    static SERIAL: Mutex<()> = Mutex::new(());

    /// Walking back and forward returns you to where you started.
    #[test]
    fn the_ring_walks_both_ways() {
        let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        fresh();
        for path in ["/a", "/b", "/c"] {
            record(path);
        }
        assert_eq!(step_back().as_deref(), Some("/b"));
        assert_eq!(step_back().as_deref(), Some("/a"));
        assert_eq!(step_back(), None, "nowhere further back");
        assert_eq!(step_forward().as_deref(), Some("/b"));
        assert_eq!(step_forward().as_deref(), Some("/c"));
        assert_eq!(step_forward(), None, "nowhere further forward");
    }

    /// Moving somewhere new after walking back abandons the forward history, as a browser does.
    #[test]
    fn a_new_directory_abandons_the_forward_history() {
        let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        fresh();
        for path in ["/a", "/b", "/c"] {
            record(path);
        }
        step_back();
        record("/d");
        assert_eq!(step_forward(), None, "there is no forward from here");
        assert_eq!(history(), ["/a", "/b", "/d"], "and /c is gone");
    }

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
}

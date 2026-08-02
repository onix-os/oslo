//! What the shell learns from being used: where you work, and what you run there.
//!
//! oslo's REPL loop already computes everything this store holds — the line, its language, whether
//! it was secret, the directory before and after, the wall-clock duration and the exit status — and
//! then discards all but the line and the language. This is not new telemetry; it is the same
//! sixty lines of `repl.rs` not throwing their inputs away. What it buys is a `cd` that can find a
//! directory by name and a suggestion that knows `cargo run --example` meant something different in
//! the last project.
//!
//! # It is an aggregate, not a log
//!
//! `history.db` is already the event log, one row per line typed. A second log would double the
//! write cost and double the privacy surface for a chronology that already exists — and turso 0.7.2
//! has neither `VACUUM` nor `auto_vacuum`, so a file that grows is a permanent high-water mark. An
//! aggregate is bounded by distinct *behaviour* rather than by time: repeats, which are the entire
//! point of the ranking, cost nothing after the first. What it gives up is the order of individual
//! executions, which is recoverable by joining `history.db` if anything ever needs it.
//!
//! # Why it lives in the library
//!
//! `src/startup/` is private to the binary (`main.rs` declares `mod startup;`), and `builtin_cd` is
//! library code that has to be able to ask this store where to jump. So it is here, and the handle
//! is a process-global installed by the interactive loop and by nothing else.
//!
//! That is the privacy design as much as the plumbing one. A script, an `oslo -c`, or a subshell
//! never calls [`install`], so [`store`] answers `None` and every clever path is *structurally*
//! unreachable rather than gated by a flag someone has to remember to check. A CI job's command
//! lines cannot reach the file because there is no file to reach.

pub mod db;
mod private;
pub mod query;
pub mod redact;
pub mod score;
pub mod write;

pub use db::{Run, Step, Track, Visit};
pub use redact::head_of;

use std::path::PathBuf;
use std::sync::OnceLock;

/// The one store, or `None` in every shell that is not an interactive session.
///
/// A `OnceLock` rather than a `Mutex` for the reason `autocd::AUTOCD` gives for its own global: this
/// is a property of the invocation, decided once before the first prompt and never afterwards.
static TRACK: OnceLock<Option<Track>> = OnceLock::new();

/// Hand the process its store. **Only the interactive loop may call this.**
///
/// `None` is a legitimate argument: a shell whose store would not open is a working shell with a
/// dumber `cd`, and installing the absence says so explicitly rather than leaving the slot to be
/// filled by whoever calls next.
///
/// Later calls are ignored, so nothing can swap the store out from under a running shell.
pub fn install(track: Option<Track>) {
    let _ = TRACK.set(track);
}

/// The store, if this shell has one.
pub fn store() -> Option<&'static Track> {
    TRACK.get()?.as_ref()
}

/// Where the store is kept, given the environment.
///
/// Beside `history.db`, and for the same reason: this is state the user accumulates, not
/// configuration they wrote. `None` when neither `$XDG_DATA_HOME` nor `$HOME` is knowable — a
/// container's `nobody` — which must run without a store rather than fail.
pub fn default_path(xdg_data: Option<&str>, home: Option<&str>) -> Option<PathBuf> {
    let base = match xdg_data {
        Some(dir) if !dir.trim().is_empty() => PathBuf::from(dir),
        _ => PathBuf::from(home?).join(".local/share"),
    };
    Some(base.join("oslo/track.db"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// The store is one process-wide slot that can only ever be written once, so every test that
    /// touches it is this one.
    static SERIAL: Mutex<()> = Mutex::new(());

    #[test]
    fn the_store_lives_beside_the_history_database() {
        assert_eq!(
            default_path(Some("/x/data"), Some("/home/u")),
            Some(PathBuf::from("/x/data/oslo/track.db"))
        );
        assert_eq!(
            default_path(None, Some("/home/u")),
            Some(PathBuf::from("/home/u/.local/share/oslo/track.db"))
        );
        // An empty XDG is unset, not a relative path from the root.
        assert_eq!(
            default_path(Some("  "), Some("/home/u")),
            Some(PathBuf::from("/home/u/.local/share/oslo/track.db"))
        );
        // Nowhere to put it is not an error; it is a shell without a store.
        assert_eq!(default_path(None, None), None);
    }

    /// The whole privacy argument in one assertion: until the interactive loop installs a store
    /// there is none, so a script cannot record anything however hard it tries.
    #[test]
    fn nothing_is_tracked_until_the_interactive_loop_installs_a_store() {
        let _guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        assert!(store().is_none(), "a process starts with no store at all");

        let dir = tempfile::tempdir().expect("a temp dir");
        let track = Track::open(&dir.path().join("track.db")).expect("the database opens");
        install(Some(track));
        assert!(store().is_some(), "and has one once it is installed");

        // A second install cannot swap the store out from under a running shell.
        install(None);
        assert!(store().is_some());
    }
}

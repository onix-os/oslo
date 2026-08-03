//! A full-screen history finder.
//!
//! Up opens it. It fills the screen with everything the shell has ever run, newest first, with a
//! search bar along the bottom; typing filters fuzzily and Enter puts the chosen line back on the
//! prompt, unrun, for you to edit or accept.
//!
//! # How it differs from the completion dropdown
//!
//! They look alike on purpose and answer different questions. The dropdown is *suggestions*: what
//! could this half-typed word become — a file, a flag, a command on `$PATH`? It appears under the
//! prompt, borrows a few rows, and is about the word the cursor is in.
//!
//! This is *history*, and only history. Nothing is suggested; every row is something you have
//! actually run. So it drops the dropdown's kind badge — every row is the same kind — and spends
//! that column on the thing worth knowing about a past command: **when you last ran it**, beside
//! how often and where.
//!
//! The finder opens over global history. Tab switches to commands from the current directory only
//! and back again; the scope shown at the end of the search bar makes that filter explicit.
//!
//! # Why it takes the whole screen
//!
//! Because history is long. The dropdown shows eight rows because a completion list that pushed
//! the prompt up the screen would be worse than the thing it is helping with. A history search is
//! the opposite: you came here to look through a lot of it, and the alternate screen means the
//! prompt is exactly where you left it when you leave.
//!
//! # Where the data comes from
//!
//! [`crate::track::history`], not the history file. The file keeps lines in the order they were
//! typed; the tracker keeps counts, timestamps and directories per command, which is what the
//! three ranking signals need. See [`rank`] for the order, and why it is that order.

pub mod rank;
pub mod render;
mod run;

/// Which part of history the finder is searching.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// Commands from every recorded directory.
    Global,
    /// Commands recorded in the shell's current directory only.
    Local,
}

pub use run::{Outcome, open};

//! Which command the finder shows first.
//!
//! Three signals, in the order they matter. The order is the whole design, so it is written down
//! rather than left in a comparator:
//!
//! 1. **How well the text matches.** Nothing else can outrank this. A finder that put a
//!    frequently-run command above a better match for what you typed would be arguing with you
//!    about what you meant, and you are the one holding the keyboard.
//! 2. **How often you have run it.** Among comparable matches, the habit wins. This is what makes
//!    `git status` beat `git stash pop` when you have typed `git s`.
//! 3. **Where you are.** A command last run in the directory you are standing in — or anywhere
//!    under it — is more likely to be the one you want than the same command from an unrelated
//!    checkout.
//!
//! Recency is the tie-break rather than a signal of its own. It decides the order of the list
//! *before* you type anything, which is when it is the only thing there is to go on; once there is
//! a query, "matches well and I run it constantly" beats "I ran something once, recently".

use crate::track::history::Command;
use crate::ui::matching::{Fuzzed, Fuzzy};

/// A command with the numbers the list is ordered by.
#[derive(Debug, Clone)]
pub struct Ranked {
    pub command: Command,
    /// The fuzzy score, or 0 when nothing was typed.
    pub score: i32,
    /// Whether the command's directory is the one the shell is in, or an ancestor of it.
    pub here: bool,
}

/// Filter `commands` by `query` and order them.
///
/// An empty query filters nothing: the finder opens showing the whole history, newest first, which
/// is the order [`crate::track::history`] already returns. That is the useful default — the thing
/// you most often want is the thing you just did.
pub fn rank(commands: &[Command], query: &str, cwd: &str, fuzzy: Fuzzy) -> Vec<Ranked> {
    if query.is_empty() {
        return commands
            .iter()
            .map(|command| Ranked {
                here: is_here(&command.dir, cwd),
                command: command.clone(),
                score: 0,
            })
            .collect();
    }

    // Folded once for the whole list rather than once per candidate. With a few thousand commands
    // and a keystroke between each pass, that difference is the frame budget.
    let pattern = Fuzzed::new(query, fuzzy);
    let mut ranked: Vec<Ranked> = commands
        .iter()
        .filter_map(|command| {
            let score = pattern.score(&command.line)?;
            Some(Ranked {
                here: is_here(&command.dir, cwd),
                command: command.clone(),
                score,
            })
        })
        .collect();

    ranked.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then(b.command.runs.cmp(&a.command.runs))
            .then(b.here.cmp(&a.here))
            .then(b.command.last_at.cmp(&a.command.last_at))
            // Total, so two openings of the same store list the same way. Without this the order
            // among equals is the hash map's, which changes between runs.
            .then(a.command.line.cmp(&b.command.line))
    });
    ranked
}

/// Whether `dir` is the working directory or contains it.
///
/// Containment rather than equality, because a command run in a repository root is still "here"
/// when you have stepped into `crates/api` — that is the same reasoning
/// `query::suggestion_in_workspace` is built on.
fn is_here(dir: &str, cwd: &str) -> bool {
    if dir.is_empty() || cwd.is_empty() {
        return false;
    }
    if dir == cwd {
        return true;
    }
    // A prefix is only an ancestor on a path boundary: `/home/me/proj` must not match
    // `/home/me/project`.
    cwd.starts_with(dir) && cwd.as_bytes().get(dir.len()) == Some(&b'/')
}

/// How long ago, in the shortest form that is still true.
///
/// The finder shows this where the completion dropdown shows a kind badge. A command is identified
/// by its own text, so the useful thing beside it is not *what* it is but *when* you last did it.
/// Rounded down and never more than two characters of number, because the column is glanced at
/// rather than read: "3d" answers the question, "3 days 4 hours ago" is a sentence.
pub fn ago(now: i64, then: i64) -> String {
    let seconds = now.saturating_sub(then);
    if seconds < 0 {
        // A clock that went backwards, or a row from the future. Neither is worth a special case
        // beyond not printing a negative.
        return "now".to_string();
    }
    const MINUTE: i64 = 60;
    const HOUR: i64 = 60 * MINUTE;
    const DAY: i64 = 24 * HOUR;
    const WEEK: i64 = 7 * DAY;
    const MONTH: i64 = 30 * DAY;
    const YEAR: i64 = 365 * DAY;

    match seconds {
        s if s < MINUTE => "now".to_string(),
        s if s < HOUR => format!("{}m", s / MINUTE),
        s if s < DAY => format!("{}h", s / HOUR),
        s if s < WEEK => format!("{}d", s / DAY),
        s if s < MONTH => format!("{}w", s / WEEK),
        s if s < YEAR => format!("{}mo", s / MONTH),
        s => format!("{}y", s / YEAR),
    }
}

#[cfg(test)]
#[path = "rank/tests.rs"]
mod tests;

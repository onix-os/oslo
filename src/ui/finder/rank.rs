//! Which command the finder shows first.
//!
//! **The kind of match decides the group; recency decides the order inside it.**
//!
//! 1. **How it matched** — see [`Quality`]. Exact, then prefix, then acronym, then word-prefix,
//!    then substring, then the scattered fuzzy fallback. Coarse on purpose: these are differences
//!    a person can see, not a number.
//! 2. **When you last ran it**, within that group.
//! 3. **How often**, then **where you were**, then the fuzzy score as a final tie-break.
//!
//! With nothing typed every row is in the same group, so the list is plain newest-first.
//!
//! # Both of the simple rules are wrong, and both were tried
//!
//! **Sorting by the match score alone** put `cd docs/`, run twice a day ago, above `cd rush`, run
//! twenty-three times two hours ago. Every row in the list already matches — the score's job
//! finished at the filter — so ordering by it means ordering by a statement about the shapes of two
//! strings, and the answer changes shape as you type.
//!
//! **Sorting by recency alone** has the opposite failure, and it is worse: a command that merely
//! shares some letters, run a minute ago, outranks one that begins with exactly what you asked for.
//! Typing the first three characters of a command you run daily and not getting it is the thing
//! that makes a finder feel broken.
//!
//! Neither signal may overrule the other, so neither is a sort key on its own. The kinds are ranked
//! and recency orders within them, which gives a rule you can hold in your head: *things that start
//! with what I typed, newest first; then things that contain it, newest first.*

use crate::track::history::Command;
use crate::ui::matching::{Fuzzed, Fuzzy, Quality};

/// A command with the numbers the list is ordered by.
#[derive(Debug, Clone)]
pub struct Ranked {
    pub command: Command,
    /// What kind of match this is — the primary sort key. See `Quality`.
    pub quality: Quality,
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
                quality: Quality::Scattered,
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
            let (quality, score) = pattern.rank(&command.line)?;
            Some(Ranked {
                here: is_here(&command.dir, cwd),
                command: command.clone(),
                quality,
                score,
            })
        })
        .collect();

    ranked.sort_by(|a, b| {
        a.quality
            .cmp(&b.quality)
            .then(b.command.last_at.cmp(&a.command.last_at))
            .then(b.command.runs.cmp(&a.command.runs))
            .then(b.here.cmp(&a.here))
            .then(b.score.cmp(&a.score))
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

//! What a line was probably meant to be, from the two things that could know.
//!
//! **`$PATH` first, the model second, and the order is the point.** `lsvlk` is a misspelling of a
//! command that exists on this machine whether or not it has ever been run here — the spelling
//! check answers it on a shell with no history at all, which is exactly when a new user is most
//! likely to typo. The model answers the other half: `cargo buidl --release` has a command word
//! that is spelled correctly, and only something that has watched you work can know what the rest
//! of it should say.
//!
//! # Why the model's answer is held to a likeness test
//!
//! [`crate::command_index::nearest`] is already bounded — two edits, or one on a short name — so
//! what it offers is a misspelling by construction. The model is not: `predict_aligned` will
//! happily answer a well-formed line with a *different* command, which is a fine prediction and a
//! terrible repair. Drawn after every line anyone types, that would be a permanent second opinion
//! rather than a correction. So a proposal has to be a plausible retyping of what is on the line,
//! and the same bounded edit distance decides it.

use crate::command_index::{self, CommandIndex};

/// The corrected line, or nothing when there is no reason to think it is wrong.
///
/// `known` answers for the names only the shell has — builtins, aliases, functions — which cannot
/// be found by looking at `$PATH`. Nothing here runs anything or touches the filesystem beyond the
/// command index, which is already built and cached for the ghost suggestion.
pub fn of(line: &str, path: &str, known: &dyn Fn(&str) -> bool) -> Option<String> {
    let typed = line.trim_end();
    if typed.trim().is_empty() {
        return None;
    }
    spelling(typed, path, known).or_else(|| learned(typed))
}

/// The command word, respelled. Only when it is a name nothing on this machine answers to.
fn spelling(line: &str, path: &str, known: &dyn Fn(&str) -> bool) -> Option<String> {
    let lead = line.len() - line.trim_start().len();
    let word = line[lead..].split_whitespace().next()?;
    // A path, an assignment or a variable is not a command name to spell-check; `./buidl` is
    // whatever the filesystem says it is, and `FOO=1 cmd` names no command in its first word.
    if word.contains('/') || word.contains('=') || word.starts_with('$') {
        return None;
    }
    if known(word) || CommandIndex::contains(path, word) {
        return None;
    }
    // **A word that begins a real command is unfinished, not wrong**, and this is what keeps the
    // check off the typing path. `nearest` measures an edit distance against every name on `$PATH`
    // — 30 µs a keystroke, against 2 µs for the whole repaint — and typing `cargo` would have paid
    // it four times on the way to a word that was never a mistake. A binary search answers first.
    if CommandIndex::has_prefix(path, word) {
        return None;
    }
    let near = command_index::nearest(path, word)?;
    Some(format!(
        "{}{}{}",
        &line[..lead],
        near,
        &line[lead + word.len()..]
    ))
}

/// The whole line, from the model, when what comes back is a retyping rather than a different idea.
fn learned(line: &str) -> Option<String> {
    let guess = oslo_base::predict::repair_here(line, 1)
        .into_iter()
        .next()?;
    plausible(line, &guess.line).then_some(guess.line)
}

/// Whether `proposal` is close enough to `typed` to be a correction of it.
///
/// The budget grows with the line because a long command has more places to slip, but slowly, and
/// a whole word swapped out never comes in under it.
///
/// **Two edits by ten characters, not one.** The commonest typo of all is a transposition —
/// `buidl`, `tset`, `stauts` — and Levenshtein charges two for it. A budget of one would reject
/// exactly the mistakes people actually make.
fn plausible(typed: &str, proposal: &str) -> bool {
    if proposal == typed {
        return false;
    }
    let budget = (typed.len() / 5).clamp(1, 3);
    if proposal.len().abs_diff(typed.len()) > budget {
        return false;
    }
    command_index::edit_distance(typed, proposal, budget).is_some_and(|found| found <= budget)
}

#[cfg(test)]
mod tests;

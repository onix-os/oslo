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
use crate::theme::{Depth, Style};

/// The names only the shell has — builtins, aliases, functions — which `$PATH` cannot answer for.
///
/// Three questions rather than one, because the check needs all three and `$PATH` is asked all
/// three: is this a name, does a name begin with it, and what are they. Answering only the first
/// is what let `sourc` be "corrected" to an unrelated program while `source` sat in the shell, and
/// what left `exitt` reaching for `edit` on `$PATH` instead of the builtin `exit`.
///
/// [`Known::all`] is called only once the cheap answers have failed, so listing them costs nothing
/// on the keystroke path.
pub struct Known<'a> {
    /// Whether this is exactly one of them.
    pub has: &'a dyn Fn(&str) -> bool,
    /// Whether any of them begins with this.
    pub begins: &'a dyn Fn(&str) -> bool,
    /// All of them.
    pub all: &'a dyn Fn() -> Vec<String>,
}

/// The corrected line, or nothing when there is no reason to think it is wrong.
///
/// Nothing here runs anything or touches the filesystem beyond the command index, which is already
/// built and cached for the ghost suggestion.
pub fn of(line: &str, path: &str, known: &Known<'_>) -> Option<String> {
    let typed = line.trim_end();
    if typed.trim().is_empty() {
        return None;
    }
    let proposal = spelling(typed, path, known)
        .or_else(|| equals_word(typed, path, known))
        .or_else(|| learned(typed))?;
    keeps_a_command_that_runs(typed, &proposal, path, known).then_some(proposal)
}

/// Refuse a proposal that would swap out a command name which already runs.
///
/// **The last word before the line is accepted, and it applies to every source.** `spelling` asks
/// this before it starts, but it was the only one that did: `learned` takes whole lines from the
/// model with no such guard, so with `tac /etc/hostname` in the history, typing `tar /etc/hostname`
/// offered `[tac] /etc/hostname` — and Right, Enter ran a different program on the same file. The
/// two names are one edit apart and both exist, which is exactly the case a distance check cannot
/// separate and this one can.
///
/// **Only the command word.** A proposal that keeps the command and repairs an argument is the
/// thing repair is for — `git psuh` to `git push` has to survive — so a valid command word is not
/// on its own a reason to say nothing. It is only a reason not to replace *it*.
fn keeps_a_command_that_runs(typed: &str, proposal: &str, path: &str, known: &Known<'_>) -> bool {
    let Some((_, was)) = command_word(typed) else {
        return true;
    };
    // A word the shell would not have read as a name anyway — a path, an assignment, `@name` — is
    // not something this can have an opinion about. `spelling` declines these for the same reason.
    if was.contains('/') || was.contains('=') || was.starts_with(['$', '@']) {
        return true;
    }
    let runs =
        (known.has)(was) || CommandIndex::contains(path, was) || oslo_base::vocab::contains(was);
    if !runs {
        return true;
    }
    command_word(proposal).is_none_or(|(_, now)| now == was)
}

/// The nearest name to `word` across `$PATH` and the shell's own names.
///
/// Both, because a misspelling does not know which kind of thing it was reaching for: `exitt` is
/// the builtin `exit`, and ranking over `$PATH` alone answered it with `edit`.
/// The same strictness [`command_index::nearest`] applies: one edit for a short name, two for a
/// long one. A shell that guesses wildly is worse than one that says nothing.
fn nearest_name(word: &str, path: &str, known: &Known<'_>) -> Option<String> {
    if word.len() < 2 {
        return None;
    }
    let budget = if word.len() <= 4 { 1 } else { 2 };
    let shell = (known.all)();
    // A transposition is what a person actually did, so it settles ties before any distance is
    // measured — the rule `nearest` follows for `$PATH`, applied to these names first because
    // `$PATH` cannot see them at all.
    if let Some(swapped) = command_index::transpositions(word).find(|w| shell.contains(w)) {
        return Some(swapped);
    }
    let from_path = command_index::nearest(path, word);
    let mut best_cost = from_path
        .as_ref()
        .and_then(|near| command_index::edit_distance(word, near, budget))
        .map_or(budget + 1, |cost| cost);
    let mut best = from_path;
    // Sorted, so two names at the same distance are separated by the alphabet rather than by
    // whatever order the environment happened to hand them over in.
    let mut shell = shell;
    shell.sort_unstable();
    for name in shell {
        if let Some(cost) = command_index::edit_distance(word, &name, budget)
            && cost < best_cost
        {
            best_cost = cost;
            best = Some(name);
        }
    }
    best
}

/// A `=name` argument, respelled.
///
/// `=name` is the shorthand for where that command lives, so its tail is a command name wherever it
/// sits on the line — `ls =somehig` is a misspelling in the same way `lsvlk` is, and the first word
/// being spelled correctly is the reason [`spelling`] never sees it.
///
/// The first wrong one wins. A line with two is a line with bigger problems than a respelling.
fn equals_word(line: &str, path: &str, known: &Known<'_>) -> Option<String> {
    let mut at = 0;
    while at < line.len() {
        let lead = line[at..].len() - line[at..].trim_start().len();
        let start = at + lead;
        let Some(word) = line[start..].split_whitespace().next() else {
            break;
        };
        if let Some(fixed) = respell_equals(word, path, known) {
            let tail = &line[start + word.len()..];
            return Some(format!("{}{fixed}{tail}", &line[..start]));
        }
        at = start + word.len();
    }
    None
}

/// `=name` with the name respelled, or nothing when it is not one or already names a command.
///
/// A name with a `/` in it is not one the shorthand resolves, and a name that begins a real command
/// is unfinished rather than wrong — the same two rules [`spelling`] follows, for the same reasons.
fn respell_equals(word: &str, path: &str, known: &Known<'_>) -> Option<String> {
    let name = word.strip_prefix('=')?;
    if name.is_empty() || name.contains('/') {
        return None;
    }
    // `=` resolves through `$PATH` alone, so a builtin is not an answer for it — but a name that
    // begins one is still somebody in the middle of typing, and not a mistake to point at.
    if CommandIndex::contains(path, name)
        || CommandIndex::has_prefix(path, name)
        || (known.begins)(name)
    {
        return None;
    }
    Some(format!("={}", command_index::nearest(path, name)?))
}

/// Where the command word starts, and what it is — skipping any `NAME=value` prefixes.
///
/// **`FOO=1 lsvlk` runs `lsvlk`**, and reading only the first word meant the correction said
/// nothing about a line the highlighter was already painting as a command that does not exist. The
/// rule is [`crate::highlight::is_assignment`]'s, shared rather than restated, because two
/// answers to "which word is the command" is what produced the disagreement.
fn command_word(line: &str) -> Option<(usize, &str)> {
    let mut at = 0;
    loop {
        let rest = line.get(at..)?;
        let lead = rest.len() - rest.trim_start().len();
        let start = at + lead;
        let word = line.get(start..)?.split_whitespace().next()?;
        if !crate::highlight::is_assignment(word) {
            return Some((start, word));
        }
        at = start + word.len();
    }
}

/// The command word, respelled. Only when it is a name nothing on this machine answers to.
fn spelling(line: &str, path: &str, known: &Known<'_>) -> Option<String> {
    let (lead, word) = command_word(line)?;
    // A path or a variable is not a command name to spell-check; `./buidl` is whatever the
    // filesystem says it is.
    //
    // Nor is a word starting with `@`: that position is reserved and `@name` does not expand there,
    // so there is no command it could have been meant to be — `@proj` was being answered with
    // "did you mean gprof?", which is a guess about a word the shell has decided not to read.
    if word.contains('/') || word.contains('=') || word.starts_with('$') || word.starts_with('@') {
        return None;
    }
    // **Grammar is not part of a name.** `(`, `{`, `}`, `)` and `!` open a subshell, a group or a
    // negation, and the word after one is the command — but the whole of `(cd` reached here as a
    // name, found `mcd` two edits away, and accepting the correction rewrote `(cd /tmp && pwd)` into
    // `mcd /tmp && pwd)`: a different program, with the subshell silently gone. Refused rather than
    // stripped, because what follows is a command the *rest* of this function would then have to
    // put back in the right place, and a correction that edits punctuation is not one anybody asked
    // for.
    if word.starts_with(['(', ')', '{', '}', '!']) {
        return None;
    }
    // A structured verb or a registered tool runs, so it is not a mistake — and `$PATH` cannot say
    // so, which is how `where` came to be "corrected" into something else.
    if (known.has)(word) || CommandIndex::contains(path, word) || oslo_base::vocab::contains(word) {
        return None;
    }
    // **A word that begins a real command is unfinished, not wrong**, and this is what keeps the
    // check off the typing path. `nearest` measures an edit distance against every name on `$PATH`
    // — 30 µs a keystroke, against 2 µs for the whole repaint — and typing `cargo` would have paid
    // it four times on the way to a word that was never a mistake. A binary search answers first.
    //
    // The shell's own names are asked too, and separately: `$PATH` has never heard of `source`, so
    // typing `sourc` was a word beginning nothing and got "corrected" to an unrelated program.
    if CommandIndex::has_prefix(path, word)
        || (known.begins)(word)
        || oslo_base::vocab::has_prefix(word)
    {
        return None;
    }
    let near = nearest_name(word, path, known)?;
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

/// The correction as it is drawn after the line: `-> [systemctl] status`.
///
/// **Only the words that changed are in brackets.** A correction shown whole makes you re-read the
/// entire line to find the one character that moved; bracketed, the difference is the thing your
/// eye lands on and the rest is there for context. That is also why the two styles are what they
/// are — the brackets carry the emphasis, so the words around them are the ordinary ghost and do
/// not compete with it.
///
/// Word-aligned by position, which is all a repair needs: a correction is a near-miss of the line,
/// so the words are already in step. Anything past the end of the typed line counts as changed.
pub fn annotate(typed: &str, fixed: &str, ghost: &Style, changed: &Style, depth: Depth) -> String {
    let before: Vec<&str> = typed.split_whitespace().collect();
    let mut out = ghost.paint("-> ", depth);
    for (at, word) in fixed.split_whitespace().enumerate() {
        if at > 0 {
            out.push_str(&ghost.paint(" ", depth));
        }
        if before.get(at) == Some(&word) {
            out.push_str(&ghost.paint(word, depth));
        } else {
            out.push_str(&changed.paint(&format!("[{word}]"), depth));
        }
    }
    out
}

#[cfg(test)]
mod tests;

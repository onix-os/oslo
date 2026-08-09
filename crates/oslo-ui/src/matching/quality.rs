//! What *kind* of match a candidate is — the coarse grouping the finder ranks by.
//!
//! Beside the scorer rather than inside it, because the two answer different questions. The scorer
//! says how good a match is, as a number. This says what sort of match it is at all, and that is
//! the one a person can see.

use super::{is_acronym, starts_word};

/// **What kind of match this is**, as a person would describe it rather than as a number.
///
/// A fuzzy score is a good tie-breaker and a bad sort key: it is a statement about the shapes of
/// two strings, and its ordering between two *different* kinds of match is arbitrary. Ranking a
/// history list by it put `cd docs/` above `cd rush` because the scorer preferred one string, and
/// the answer changed shape as you typed.
///
/// Ranking by recency alone has the opposite failure, and it is the worse one: a scattered match
/// typed a minute ago outranked a command beginning with exactly what you asked for. Typing the
/// first three characters of something you run daily and not getting it is what makes a finder
/// feel broken.
///
/// So the kinds are ordered, coarsely, and recency decides *within* a kind. That gives a rule you
/// can hold in your head — "things that start with what I typed, newest first; then things that
/// contain it, newest first" — and neither signal overrules the other.
///
/// Declaration order is the ranking; `Ord` is derived from it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Quality {
    /// The whole line is what was typed.
    Exact,
    /// The line begins with it: `car` on `cargo test`.
    Prefix,
    /// The initials of consecutive words: `gco` on `git checkout origin`.
    ///
    /// Above `WordPrefix` because an acronym is deliberate. Nobody types `gco` by accident, and the
    /// command it names is almost always the one they meant.
    Acronym,
    /// A word begins with it: `test` on `cargo test`.
    WordPrefix,
    /// It appears somewhere, unbroken: `argo` on `cargo`.
    Substring,
    /// Matched only by letting the letters spread out. The fallback, and the one that must never
    /// outrank the others however recent it is.
    Scattered,
}

impl Quality {
    /// Classify an already-folded candidate against an already-folded pattern.
    ///
    /// Both are lowercase `char` slices because the caller folded them once for the whole list;
    /// folding here would be a `String` per candidate per keystroke.
    pub(super) fn of(have: &[char], wanted: &[char]) -> Quality {
        if wanted.is_empty() {
            return Quality::Scattered;
        }
        if have == wanted {
            return Quality::Exact;
        }
        if have.starts_with(wanted) {
            return Quality::Prefix;
        }
        if is_acronym(have, wanted) {
            return Quality::Acronym;
        }
        if (0..have.len())
            .filter(|&i| starts_word(have, i))
            .any(|i| have[i..].starts_with(wanted))
        {
            return Quality::WordPrefix;
        }
        if have.windows(wanted.len()).any(|w| w == wanted) {
            return Quality::Substring;
        }
        Quality::Scattered
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(candidate: &str, typed: &str) -> Quality {
        let have: Vec<char> = candidate.chars().flat_map(char::to_lowercase).collect();
        let wanted: Vec<char> = typed.chars().flat_map(char::to_lowercase).collect();
        Quality::of(&have, &wanted)
    }

    /// The six kinds, each on the case it exists for.
    #[test]
    fn each_kind_is_recognised() {
        assert_eq!(q("cargo test", "cargo test"), Quality::Exact);
        assert_eq!(q("cargo test", "car"), Quality::Prefix);
        assert_eq!(q("git checkout origin", "gco"), Quality::Acronym);
        assert_eq!(q("cargo test", "test"), Quality::WordPrefix);
        assert_eq!(q("cargo", "argo"), Quality::Substring);
        assert_eq!(q("codex --always-run", "car"), Quality::Scattered);
    }

    /// Case never decides the kind: shells are typed in a hurry.
    #[test]
    fn the_kinds_ignore_case() {
        assert_eq!(q("CARGO test", "car"), Quality::Prefix);
        assert_eq!(q("Git CheckOut", "gc"), Quality::Acronym);
        assert_eq!(q("Cargo Test", "cargo test"), Quality::Exact);
    }

    /// **An acronym may not skip a word.** `gco` is `git checkout origin`; `gio` would have to jump
    /// over `checkout`, and a query that skips words is a scatter — which is what the last kind is
    /// for.
    #[test]
    fn an_acronym_may_not_skip_a_word() {
        assert_eq!(q("git checkout origin", "gco"), Quality::Acronym);
        assert_eq!(q("git checkout origin", "gc"), Quality::Acronym);
        assert_ne!(q("git checkout origin", "gio"), Quality::Acronym);
    }

    /// The separators are the ones a command line actually uses, so a path, a flag or a host has
    /// word starts where you would point at them.
    #[test]
    fn a_word_starts_after_a_shell_separator() {
        assert_eq!(q("cargo run --example", "example"), Quality::WordPrefix);
        assert_eq!(q("src/ui/matching.rs", "matching"), Quality::WordPrefix);
        assert_eq!(q("ssh deploy@build-01", "build"), Quality::WordPrefix);
        assert_eq!(q("TERM=dumb oslo", "dumb"), Quality::WordPrefix);
    }

    /// Ordering is the declaration order, and that order is the whole design.
    #[test]
    fn better_kinds_sort_first() {
        let mut kinds = [
            Quality::Scattered,
            Quality::Prefix,
            Quality::Substring,
            Quality::Exact,
            Quality::WordPrefix,
            Quality::Acronym,
        ];
        kinds.sort();
        assert_eq!(
            kinds,
            [
                Quality::Exact,
                Quality::Prefix,
                Quality::Acronym,
                Quality::WordPrefix,
                Quality::Substring,
                Quality::Scattered,
            ]
        );
    }

    /// Nothing typed is not a match of any kind — the finder shows the whole list instead.
    #[test]
    fn an_empty_query_is_not_a_kind() {
        assert_eq!(q("anything", ""), Quality::Scattered);
    }
}

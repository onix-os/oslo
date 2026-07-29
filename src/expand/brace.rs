//! Brace expansion: `a{1,2}b` -> `a1b a2b`, `{1..5}` -> `1 2 3 4 5`.
//!
//! This runs *before* every other expansion, because it is the only one that turns one word into
//! several words rather than one word into several fields. `mkdir -p build/{bin,lib}` must reach
//! `mkdir` as two arguments; no amount of later field splitting can recover that, since splitting
//! only ever cuts the *result* of an expansion on IFS.
//!
//! It operates on the word's parts rather than on its source text, because quoting has already
//! been resolved by then and quoting is what decides whether a brace is syntax: `"{a,b}"` and
//! `{a\,b}` are literal, `{a,b}` is not. A [`WordPart`] that is not unquoted literal text is
//! therefore opaque here — it can neither open a group nor separate alternatives — while every
//! character of a [`WordPart::Literal`] is a candidate piece of brace syntax.
//!
//! Anything that does not parse as a group stays exactly as it was typed. That is not a fallback,
//! it is the specification: `echo {a}` prints `{a}`, and a shell that guessed otherwise would
//! quietly rewrite awk programs and JSON literals.
//!
//! Working on parts rather than text costs one known deviation from bash, whose brace expansion is
//! textual and therefore able to fuse a group boundary into a *name*: bash reads `{$v,y}z` as the
//! two words `$vz` and `yz`, so the first names a variable that does not exist. Here `$v` is
//! already its own part and the result is `<v>z`. Recovering bash's answer means expanding braces
//! on source text before the word is lexed, which is a parser change, not an expansion one.

use crate::ast::{Word, WordPart};

/// Ceiling on how many items one `{n..m}` may generate.
///
/// A range is written by a human but its bounds can come from a typo (`{1..999999999}`), and the
/// items are materialised in memory before anything can consume them. Refusing to expand — which
/// leaves the text literal, the same answer as any other malformed brace group — is a far better
/// failure than an allocation that takes the shell down with it.
const SEQUENCE_LIMIT: i128 = 100_000;

/// One position in a word, as brace expansion sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Atom {
    /// A character of unquoted literal text, and so a possible `{`, `,` or `}`.
    Raw(char),
    /// A quoted run or an expansion. Opaque: it survives into the output untouched.
    Part(WordPart),
}

/// Expand the brace groups in `word`, yielding one word per combination.
///
/// A word with no expandable group comes back as itself, which is the overwhelmingly common case
/// and the reason for the cheap `{` scan up front.
pub fn expand_braces(word: &Word) -> Vec<Word> {
    let has_brace = word
        .parts
        .iter()
        .any(|p| matches!(p, WordPart::Literal(s) if s.contains('{')));
    if !has_brace {
        return vec![word.clone()];
    }

    let atoms = to_atoms(word);
    let expanded = expand_atoms(&atoms);
    // Nothing expandable: hand back the original word rather than a rebuilt equal one, so a word
    // that merely mentions a brace is not paying for a reconstruction.
    if expanded.len() == 1 && expanded[0] == atoms {
        return vec![word.clone()];
    }
    expanded.iter().map(|a| from_atoms(a)).collect()
}

fn to_atoms(word: &Word) -> Vec<Atom> {
    let mut atoms = Vec::new();
    for part in &word.parts {
        match part {
            WordPart::Literal(s) => atoms.extend(s.chars().map(Atom::Raw)),
            other => atoms.push(Atom::Part(other.clone())),
        }
    }
    atoms
}

fn from_atoms(atoms: &[Atom]) -> Word {
    let mut parts: Vec<WordPart> = Vec::new();
    let mut lit = String::new();
    for atom in atoms {
        match atom {
            Atom::Raw(c) => lit.push(*c),
            Atom::Part(p) => {
                if !lit.is_empty() {
                    parts.push(WordPart::Literal(std::mem::take(&mut lit)));
                }
                parts.push(p.clone());
            }
        }
    }
    if !lit.is_empty() {
        parts.push(WordPart::Literal(lit));
    }
    Word { parts }
}

fn is_raw(atom: &Atom, ch: char) -> bool {
    matches!(atom, Atom::Raw(c) if *c == ch)
}

/// Expand the leftmost expandable group, then recurse into what it produced.
///
/// "Leftmost *expandable*" is doing real work: `{a}{b,c}` has an earlier brace pair that is not a
/// group, and bash still expands the later one, giving `{a}b {a}c`. So a pair that turns out not
/// to be a group is skipped rather than ending the search.
fn expand_atoms(atoms: &[Atom]) -> Vec<Vec<Atom>> {
    for open in 0..atoms.len() {
        if !is_raw(&atoms[open], '{') {
            continue;
        }
        let Some(close) = matching_close(atoms, open) else {
            // An unmatched `{` is literal, but a group can still open *inside* it:
            // `{a{b,c}` is `{ab {ac`.
            continue;
        };
        let inner = &atoms[open + 1..close];
        let Some(alternatives) = comma_alternatives(inner).or_else(|| sequence_alternatives(inner))
        else {
            continue;
        };

        let prefix = &atoms[..open];
        let suffixes = expand_atoms(&atoms[close + 1..]);
        let mut out = Vec::new();
        for alternative in alternatives {
            // An alternative may itself contain groups: `{a,b{c,d}}` is `a bc bd`.
            for body in expand_atoms(&alternative) {
                for suffix in &suffixes {
                    let mut word = Vec::with_capacity(prefix.len() + body.len() + suffix.len());
                    word.extend_from_slice(prefix);
                    word.extend_from_slice(&body);
                    word.extend_from_slice(suffix);
                    out.push(word);
                }
            }
        }
        return out;
    }
    vec![atoms.to_vec()]
}

/// Index of the `}` closing the `{` at `open`, honouring nesting.
fn matching_close(atoms: &[Atom], open: usize) -> Option<usize> {
    let mut depth = 0usize;
    for (i, atom) in atoms.iter().enumerate().skip(open) {
        if is_raw(atom, '{') {
            depth += 1;
        } else if is_raw(atom, '}') {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
    }
    None
}

/// Split a group body on its top-level commas, or `None` if it has none.
///
/// No comma means this is not a comma list, and the caller then tries a sequence expression before
/// giving up. Empty alternatives are real: `a{,}b` is `ab ab`.
fn comma_alternatives(inner: &[Atom]) -> Option<Vec<Vec<Atom>>> {
    let mut parts: Vec<Vec<Atom>> = vec![Vec::new()];
    let mut depth = 0usize;
    for atom in inner {
        if is_raw(atom, ',') && depth == 0 {
            parts.push(Vec::new());
            continue;
        }
        if is_raw(atom, '{') {
            depth += 1;
        } else if is_raw(atom, '}') {
            depth = depth.saturating_sub(1);
        }
        parts
            .last_mut()
            .expect("parts always holds the open alternative")
            .push(atom.clone());
    }
    (parts.len() > 1).then_some(parts)
}

/// Expand `{n..m}`, `{n..m..step}` and their single-character form `{a..e}`.
fn sequence_alternatives(inner: &[Atom]) -> Option<Vec<Vec<Atom>>> {
    // A sequence expression is pure text; a `$x` anywhere in it means this is not one.
    let text = inner
        .iter()
        .map(|a| match a {
            Atom::Raw(c) => Some(*c),
            Atom::Part(_) => None,
        })
        .collect::<Option<String>>()?;

    let mut fields = text.split("..");
    let start = fields.next()?;
    let end = fields.next()?;
    let step = fields.next();
    if fields.next().is_some() {
        return None;
    }
    let step = match step {
        // bash takes the magnitude and gets direction from the endpoints, so `{1..5..-2}` counts
        // up; a zero step is read as one rather than as a sequence that never terminates.
        Some(s) => s.parse::<i64>().ok()?.checked_abs()?.max(1),
        None => 1,
    };

    let items = numeric_sequence(start, end, step).or_else(|| char_sequence(start, end, step))?;
    Some(
        items
            .into_iter()
            .map(|s| s.chars().map(Atom::Raw).collect())
            .collect(),
    )
}

/// Whether an operand asks for zero padding, i.e. it has a leading zero of its own.
///
/// `{01..10}` is `01 02 ... 10`; `{1..10}` is not padded.
fn is_zero_padded(s: &str) -> bool {
    let digits = s
        .strip_prefix('-')
        .or_else(|| s.strip_prefix('+'))
        .unwrap_or(s);
    digits.len() > 1 && digits.starts_with('0')
}

fn numeric_sequence(start: &str, end: &str, step: i64) -> Option<Vec<String>> {
    let from: i64 = start.parse().ok()?;
    let to: i64 = end.parse().ok()?;
    // bash counts the sign into the field width: `{-01..1}` is `-01 000 001`.
    let width = if is_zero_padded(start) || is_zero_padded(end) {
        start.len().max(end.len())
    } else {
        0
    };

    let count = (i128::from(to) - i128::from(from)).abs() / i128::from(step) + 1;
    if count > SEQUENCE_LIMIT {
        return None;
    }

    let mut out = Vec::with_capacity(count as usize);
    for i in 0..count {
        let v = if from <= to {
            i128::from(from) + i * i128::from(step)
        } else {
            i128::from(from) - i * i128::from(step)
        };
        out.push(format!("{v:0width$}"));
    }
    Some(out)
}

fn char_sequence(start: &str, end: &str, step: i64) -> Option<Vec<String>> {
    let from = i64::from(single_ascii(start)?);
    let to = i64::from(single_ascii(end)?);

    // Counting first keeps the arithmetic bounded: the largest multiple of `step` used is at most
    // the distance between the endpoints, so an absurd step just yields the single first item.
    let count = (to - from).abs() / step + 1;
    let mut out = Vec::with_capacity(count as usize);
    for i in 0..count {
        let c = if from <= to {
            from + i * step
        } else {
            from - i * step
        };
        out.push(char::from(u8::try_from(c).ok()?).to_string());
    }
    Some(out)
}

/// A sequence endpoint written as one ASCII letter, as in `{a..z}`.
///
/// Letters only: `{1..z}` is not a range in bash either, and reading it as one would turn a
/// mistyped numeric range into a stream of punctuation.
fn single_ascii(s: &str) -> Option<u8> {
    let mut chars = s.chars();
    let c = chars.next()?;
    if chars.next().is_some() || !c.is_ascii_alphabetic() {
        return None;
    }
    Some(c as u8)
}

#[cfg(test)]
mod tests {
    use super::expand_braces;
    use crate::ast::{ParamExpansion, Word, WordPart};

    /// Expand a word written as plain unquoted text, the shape most cases have.
    fn expand(text: &str) -> Vec<String> {
        expand_braces(&Word::from_literal(text))
            .iter()
            .map(|w| {
                w.parts
                    .iter()
                    .map(|p| match p {
                        WordPart::Literal(s) => s.clone(),
                        other => panic!("unexpected part {other:?}"),
                    })
                    .collect()
            })
            .collect()
    }

    #[test]
    fn comma_list_expands_with_prefix_and_suffix() {
        assert_eq!(expand("x{a,b}y"), vec!["xay", "xby"]);
    }

    #[test]
    fn empty_alternatives_are_real_alternatives() {
        assert_eq!(expand("a{,}b"), vec!["ab", "ab"]);
        assert_eq!(expand("a{b,}"), vec!["ab", "a"]);
    }

    #[test]
    fn adjacent_groups_multiply() {
        assert_eq!(expand("{a,b}{1,2}"), vec!["a1", "a2", "b1", "b2"]);
    }

    #[test]
    fn groups_nest() {
        assert_eq!(expand("{a,b{c,d}}"), vec!["a", "bc", "bd"]);
    }

    /// A brace pair with no comma is not a group, and must not stop the search for a later one.
    #[test]
    fn non_group_braces_stay_literal() {
        assert_eq!(expand("a{b}c"), vec!["a{b}c"]);
        assert_eq!(expand("{}"), vec!["{}"]);
        assert_eq!(expand("{a}{b,c}"), vec!["{a}b", "{a}c"]);
    }

    #[test]
    fn unmatched_braces_stay_literal() {
        assert_eq!(expand("{a,b"), vec!["{a,b"]);
        assert_eq!(expand("}a{"), vec!["}a{"]);
        // The outer `{` never closes, but the inner group is still a group.
        assert_eq!(expand("{a{b,c}"), vec!["{ab", "{ac"]);
    }

    #[test]
    fn numeric_ranges_count_both_ways() {
        assert_eq!(expand("{1..4}"), vec!["1", "2", "3", "4"]);
        assert_eq!(expand("{4..1}"), vec!["4", "3", "2", "1"]);
        assert_eq!(expand("{-2..2}"), vec!["-2", "-1", "0", "1", "2"]);
    }

    #[test]
    fn range_step_is_a_magnitude() {
        assert_eq!(expand("{0..10..3}"), vec!["0", "3", "6", "9"]);
        // A negative step is not a direction: the endpoints already gave one.
        assert_eq!(expand("{1..5..-2}"), vec!["1", "3", "5"]);
        // bash reads a zero step as one, rather than as a group it refuses to expand.
        assert_eq!(expand("{1..3..0}"), vec!["1", "2", "3"]);
    }

    #[test]
    fn leading_zero_pads_the_whole_range() {
        assert_eq!(expand("{08..11}"), vec!["08", "09", "10", "11"]);
        assert_eq!(expand("{-01..1}"), vec!["-01", "000", "001"]);
    }

    #[test]
    fn character_ranges_walk_the_alphabet() {
        assert_eq!(expand("{a..e}"), vec!["a", "b", "c", "d", "e"]);
        assert_eq!(expand("{e..a..2}"), vec!["e", "c", "a"]);
    }

    #[test]
    fn malformed_ranges_stay_literal() {
        assert_eq!(expand("{1...5}"), vec!["{1...5}"]);
        assert_eq!(expand("{1..z}"), vec!["{1..z}"]);
        assert_eq!(expand("{a..b..c}"), vec!["{a..b..c}"]);
        // Refusing an absurd range is the same answer as any other unparsable group.
        assert_eq!(expand("{1..99999999}"), vec!["{1..99999999}"]);
    }

    #[test]
    fn words_without_braces_are_returned_untouched() {
        let w = Word::from_literal("plain");
        assert_eq!(expand_braces(&w), vec![w]);
    }

    /// Quoted text is not brace syntax, and an expansion inside a group survives into each
    /// alternative as an expansion rather than being flattened to text.
    #[test]
    fn quoted_braces_are_literal_and_expansions_are_carried_along() {
        let quoted = Word {
            parts: vec![WordPart::SingleQuoted("{a,b}".into())],
        };
        assert_eq!(expand_braces(&quoted), vec![quoted.clone()]);

        let var = WordPart::Variable {
            name: "x".into(),
            expansion_type: ParamExpansion::Normal,
        };
        let w = Word {
            parts: vec![
                WordPart::Literal("{".into()),
                var.clone(),
                WordPart::Literal(",b}".into()),
            ],
        };
        let got = expand_braces(&w);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].parts, vec![var]);
        assert_eq!(got[1].parts, vec![WordPart::Literal("b".into())]);
    }

    /// A comma that only exists because of an expansion does not split the group: the group is
    /// delimited by syntax, not by the text an expansion happens to produce.
    #[test]
    fn a_comma_from_an_expansion_does_not_split() {
        let w = Word {
            parts: vec![
                WordPart::Literal("{a".into()),
                WordPart::SingleQuoted(",".into()),
                WordPart::Literal("b}".into()),
            ],
        };
        assert_eq!(expand_braces(&w), vec![w.clone()]);
    }
}

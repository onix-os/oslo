//! Compiling and matching one shell pattern.
//!
//! Split out from pathname expansion because the same dialect answers four different questions —
//! `case`, `[[ x == p ]]`, the `${v#p}` family and the filesystem walk — and only the last of them
//! is about paths. What they share is the thing that makes a *shell* pattern different from a
//! glob: **quoting decides, per character, whether a metacharacter is one**.
//!
//! That is not a detail. `case $answer in "$expected") ok ;; esac` is an ordinary way to compare
//! two strings, and a matcher that reads the expanded `$expected` as a pattern answers "yes" to
//! `case anything in "*")`. Every caller therefore compiles from [`Run`]s, which still carry the
//! quoting, rather than from a flat `String` that has forgotten it.

use crate::expand::word::Run;

/// One element of a compiled pattern.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Item {
    /// A character that must match itself — either typed literally or quoted into submission.
    Ch(char),
    /// `?`: exactly one character.
    Any,
    /// `*`: any run of characters.
    Star,
    /// `[abc]`, `[a-z]`, `[!abc]`.
    Class { negated: bool, members: Vec<Member> },
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Member {
    Ch(char),
    Range(char, char),
    /// A POSIX character class: the `alpha` of `[[:alpha:]]`.
    Named(String),
}

/// Whether `ch` belongs to the POSIX character class `name`.
///
/// An unknown name matches nothing rather than aborting: a shell has no way to report a bad
/// pattern, and the word will fall back to its literal text if this was its only chance to match.
fn in_named_class(name: &str, ch: char) -> bool {
    match name {
        "alnum" => ch.is_alphanumeric(),
        "alpha" => ch.is_alphabetic(),
        "blank" => ch == ' ' || ch == '\t',
        "cntrl" => ch.is_control(),
        "digit" => ch.is_ascii_digit(),
        "graph" => !ch.is_control() && !ch.is_whitespace(),
        "lower" => ch.is_lowercase(),
        "print" => !ch.is_control(),
        "punct" => ch.is_ascii_punctuation(),
        "space" => ch.is_whitespace(),
        "upper" => ch.is_uppercase(),
        "xdigit" => ch.is_ascii_hexdigit(),
        _ => false,
    }
}

impl Item {
    pub(super) fn matches_char(&self, ch: char) -> bool {
        match self {
            Item::Ch(c) => *c == ch,
            Item::Any => true,
            // `*` is handled by the outer loop; it never consumes a single character on its own.
            Item::Star => false,
            Item::Class { negated, members } => {
                let hit = members.iter().any(|m| match m {
                    Member::Ch(c) => *c == ch,
                    Member::Range(lo, hi) => *lo <= ch && ch <= *hi,
                    Member::Named(name) => in_named_class(name, ch),
                });
                hit != *negated
            }
        }
    }
}

/// Compile a run of `(character, is-it-a-metacharacter)` pairs, reporting whether any character
/// turned out to be a metacharacter.
///
/// The flag is what lets pathname expansion skip the directory walk for a word that only *looks*
/// like a pattern, and what makes an unterminated `[` fall back to literal text.
pub(super) fn compile_items(chars: &[(char, bool)]) -> (Vec<Item>, bool) {
    let mut items = Vec::new();
    let mut has_metacharacter = false;
    let mut i = 0;

    while i < chars.len() {
        let (ch, globs) = chars[i];
        if globs {
            match ch {
                '*' => {
                    // POSIX has no globstar: `**` is just `*`, and two adjacent stars would only
                    // make the matcher backtrack for nothing.
                    if items.last() != Some(&Item::Star) {
                        items.push(Item::Star);
                    }
                    has_metacharacter = true;
                    i += 1;
                    continue;
                }
                '?' => {
                    items.push(Item::Any);
                    has_metacharacter = true;
                    i += 1;
                    continue;
                }
                '[' => {
                    if let Some((class, next)) = parse_class(chars, i) {
                        items.push(class);
                        has_metacharacter = true;
                        i = next;
                        continue;
                    }
                }
                _ => {}
            }
        }
        items.push(Item::Ch(ch));
        i += 1;
    }

    (items, has_metacharacter)
}

/// Parse `[...]` starting at `start`, or return `None` when it is never closed.
///
/// An unclosed `[` is a literal `[` — `echo a[` prints `a[` rather than failing — so the caller
/// needs to be able to back out. A `]` in the first position is a member, not the terminator.
fn parse_class(chars: &[(char, bool)], start: usize) -> Option<(Item, usize)> {
    let mut i = start + 1;
    let mut negated = false;
    if let Some(&(ch, _)) = chars.get(i)
        && (ch == '!' || ch == '^')
    {
        negated = true;
        i += 1;
    }

    let mut members = Vec::new();
    while i < chars.len() {
        let (ch, globs) = chars[i];
        // A quoted `]` is data; only an unquoted one can close the class.
        if ch == ']' && globs && !members.is_empty() {
            return Some((Item::Class { negated, members }, i + 1));
        }
        // `[[:digit:]]` nests a named class inside the bracket, so its `]` is not the closer.
        if let Some((name, next)) = parse_named_class(chars, i) {
            members.push(Member::Named(name));
            i = next;
            continue;
        }
        // `a-z` is a range, but the `-` in `[a-]` is an ordinary member.
        if let (Some(&('-', true)), Some(&(hi, hi_globs))) = (chars.get(i + 1), chars.get(i + 2))
            && !(hi == ']' && hi_globs)
        {
            members.push(Member::Range(ch, hi));
            i += 3;
            continue;
        }
        members.push(Member::Ch(ch));
        i += 1;
    }
    None
}

/// Parse `[:name:]` at `start`, returning the name and the index just past the closing `]`.
fn parse_named_class(chars: &[(char, bool)], start: usize) -> Option<(String, usize)> {
    if chars.get(start)?.0 != '[' || chars.get(start + 1)?.0 != ':' {
        return None;
    }
    let mut name = String::new();
    let mut i = start + 2;
    while let Some(&(ch, _)) = chars.get(i) {
        if ch == ':' && chars.get(i + 1).map(|c| c.0) == Some(']') {
            return Some((name, i + 2));
        }
        name.push(ch);
        i += 1;
    }
    None
}

/// Whether the whole of `name` matches a compiled item list.
///
/// Backtracking on the most recent `*` only: shell patterns have no alternation, so one
/// resumption point is enough and the match stays linear in practice.
pub(super) fn matches_items(items: &[Item], name: &str) -> bool {
    let name: Vec<char> = name.chars().collect();
    let (mut i, mut j) = (0, 0);
    // The most recent `*` and how much of the name it had swallowed, for backtracking.
    let mut star: Option<(usize, usize)> = None;

    while j < name.len() {
        match items.get(i) {
            Some(Item::Star) => {
                star = Some((i, j));
                i += 1;
            }
            Some(item) if item.matches_char(name[j]) => {
                i += 1;
                j += 1;
            }
            _ => match star {
                // Let the star eat one more character and try the rest of the pattern again.
                Some((star_index, eaten)) => {
                    i = star_index + 1;
                    j = eaten + 1;
                    star = Some((star_index, j));
                }
                None => return false,
            },
        }
    }

    while items.get(i) == Some(&Item::Star) {
        i += 1;
    }
    i == items.len()
}

/// A compiled shell pattern, ready to be matched against as many strings as the caller likes.
///
/// `${v##*/}` tries every prefix of the value and `${v//p/r}` every position in it, so compiling
/// once and matching many times is not an optimisation but the difference between a linear and a
/// quadratic operator.
#[derive(Debug)]
pub struct ShellPattern {
    items: Vec<Item>,
}

impl ShellPattern {
    /// Compile from expanded runs, honouring the quoting each run carries.
    ///
    /// This is the entry point every shell-level caller should use: it is what makes
    /// `case $x in "$p")` a string comparison and `case $x in $p)` a pattern match.
    pub fn from_runs(runs: &[Run]) -> Self {
        let chars: Vec<(char, bool)> = runs
            .iter()
            .flat_map(|run| run.text.chars().map(move |ch| (ch, run.globs())))
            .collect();
        Self {
            items: compile_items(&chars).0,
        }
    }

    /// Compile text in which every metacharacter is meant as one.
    ///
    /// For callers that never had the quoting to begin with — `[[ x == p ]]` receives its right
    /// operand as an already-flattened string — and for tests.
    pub fn from_unquoted(text: &str) -> Self {
        let chars: Vec<(char, bool)> = text.chars().map(|ch| (ch, true)).collect();
        Self {
            items: compile_items(&chars).0,
        }
    }

    /// Does the whole of `text` match?
    pub fn matches(&self, text: &str) -> bool {
        matches_items(&self.items, text)
    }
}

#[cfg(test)]
mod tests {
    use super::ShellPattern;
    use crate::expand::word::{Origin, Run};

    fn quoted_then(literal: &str, pattern: &str) -> ShellPattern {
        ShellPattern::from_runs(&[
            Run::new(literal, Origin::Quoted),
            Run::new(pattern, Origin::Literal),
        ])
    }

    #[test]
    fn quoting_turns_a_metacharacter_into_a_character() {
        // The defect that motivated compiling from runs: `case abc in "*")` matched everything.
        let quoted = ShellPattern::from_runs(&[Run::new("*", Origin::Quoted)]);
        assert!(quoted.matches("*"));
        assert!(!quoted.matches("abc"));

        let unquoted = ShellPattern::from_unquoted("*");
        assert!(unquoted.matches("abc"));
    }

    /// One word can be part quoted and part not, and each part keeps its own answer.
    #[test]
    fn quoting_is_decided_per_character() {
        let p = quoted_then("a*", "?");
        assert!(p.matches("a*z"));
        assert!(!p.matches("abz"));
    }

    /// A `]` that arrived quoted is a member of the bracket expression, not its terminator.
    /// modernish rejects a shell that gets this wrong before it will run at all.
    #[test]
    fn a_quoted_bracket_close_is_a_member() {
        let p = ShellPattern::from_runs(&[
            Run::new("*[", Origin::Literal),
            Run::new("ab]cd", Origin::Quoted),
            Run::new("]*", Origin::Literal),
        ]);
        assert!(p.matches("c"));
        assert!(p.matches("x]y"));
        assert!(!p.matches("z"));
    }

    #[test]
    fn an_unterminated_bracket_is_literal_text() {
        let p = ShellPattern::from_unquoted("a[b");
        assert!(p.matches("a[b"));
        assert!(!p.matches("ab"));
    }

    #[test]
    fn classes_ranges_and_negation() {
        assert!(ShellPattern::from_unquoted("[a-c]").matches("b"));
        assert!(!ShellPattern::from_unquoted("[a-c]").matches("d"));
        assert!(ShellPattern::from_unquoted("[!a-c]").matches("d"));
        assert!(ShellPattern::from_unquoted("[[:digit:]]").matches("7"));
        assert!(!ShellPattern::from_unquoted("[[:digit:]]").matches("x"));
        // `-` last in the bracket is a member, not the start of a range.
        assert!(ShellPattern::from_unquoted("[a-]").matches("-"));
    }

    /// `/` is an ordinary character here: only pathname expansion stops a `*` at one.
    #[test]
    fn a_star_crosses_a_slash() {
        assert!(ShellPattern::from_unquoted("*/*").matches("a/b/c"));
        assert!(ShellPattern::from_unquoted("*").matches("a/b"));
    }

    /// And so is a leading dot: `case .git in *)` matches, however much `echo *` must not.
    #[test]
    fn a_leading_dot_is_not_special() {
        assert!(ShellPattern::from_unquoted("*").matches(".git"));
        assert!(ShellPattern::from_unquoted("?git").matches(".git"));
    }

    #[test]
    fn the_empty_pattern_matches_only_the_empty_string() {
        assert!(ShellPattern::from_unquoted("").matches(""));
        assert!(!ShellPattern::from_unquoted("").matches("x"));
        assert!(ShellPattern::from_unquoted("*").matches(""));
    }
}

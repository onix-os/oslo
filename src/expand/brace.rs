//! Brace expansion: `a{1,2}b` -> `a1b a2b`, `{1..5}` -> `1 2 3 4 5`.
//!
//! This is the only expansion that turns one word into several *words* rather than one word into
//! several fields. `mkdir -p build/{bin,lib}` must reach `mkdir` as two arguments, and no amount
//! of later field splitting can recover that, since splitting only ever cuts the *result* of an
//! expansion on IFS.
//!
//! It runs on the word's **source text**, before the word is lexed, which is where bash runs it
//! and is the whole reason it lives on this side of the lexer. A brace group is a piece of
//! *lexical* syntax, not a boundary between already-formed word parts, so it can fuse the text on
//! either side of itself into a single token:
//!
//! ```text
//! v=x; echo {$v,y}z    ->  $vz  yz    the group's suffix extends the *name* `$v`, so `$vz` is unset
//! v=x; echo $v{a,b}    ->  $va  $vb   and so does an alternative, when the name comes first
//! v=x; echo ${v}{a,b}  ->  xa   xb    `${v}` closes the name, so here the fusion cannot happen
//! ```
//!
//! Expanding over word *parts* instead gets the third line right and the first two wrong, because
//! by then `$v` is a part of its own and can never grow a `z`. That was rush's behaviour until
//! this pass moved ahead of the lexer.
//!
//! Because the text is still unlexed, this module has to decide for itself which characters are
//! syntax — quoting is exactly what says whether a brace is a brace (`"{a,b}"` and `{a\,b}` are
//! literal), and an expansion is opaque even when it contains a comma (`${x:-a,b}` is one word).
//! `to_atoms` is that decision, and it is the only place in this file that looks at a character
//! for any reason other than brace syntax.
//!
//! Anything that does not parse as a group stays exactly as it was typed. That is not a fallback,
//! it is the specification: `echo {a}` prints `{a}`, and a shell that guessed otherwise would
//! quietly rewrite awk programs and JSON literals.
//!
//! This file owns the syntax — what is a group, where it ends, how groups combine. The `sequence`
//! module beside it owns what a `{n..m}` denotes.

mod sequence;

use sequence::sequence_alternatives;

/// Ceiling on how many items one `{n..m}` may generate.
///
/// A range is written by a human but its bounds can come from a typo (`{1..999999999}`), and the
/// items are materialised in memory before anything can consume them. Refusing to expand — which
/// leaves the text literal, the same answer as any other malformed brace group — is a far better
/// failure than an allocation that takes the shell down with it.
const SEQUENCE_LIMIT: i128 = 100_000;

/// Ceiling on how many words one word's brace expansion may produce in total.
///
/// [`SEQUENCE_LIMIT`] bounds a single range; this bounds their product, which is what
/// `{1..1000}{1..1000}` multiplies out to. It matters more here than it would inside the
/// evaluator, because this pass runs while the *parser* is building the command — so without it
/// `if false; then echo {1..100000}{1..100000}; fi` wedges the shell on a branch it was never
/// going to take, and no `if` guarding the line can save it. Over budget the word is left exactly
/// as it was typed, the same answer this pass gives any other group it declines.
const WORD_LIMIT: usize = 100_000;

/// One position in a word, as brace expansion sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Atom {
    /// A character that is exposed to brace syntax, and so a possible `{`, `,` or `}`.
    Raw(char),
    /// A quoted run or an expansion, carried through verbatim. Opaque: the characters inside it
    /// are text for the lexer that runs after this pass, never brace syntax here.
    Opaque(String),
}

/// Expand the brace groups in the source text of one word, one output word per combination.
///
/// A word with no expandable group comes back as itself, which is the overwhelmingly common case
/// and the reason for the cheap `{` scan up front.
pub fn expand_braces_text(word: &str) -> Vec<String> {
    if !word.contains('{') {
        return vec![word.to_string()];
    }

    let atoms = to_atoms(word);
    let expanded = expand_atoms(&atoms);
    // Nothing expandable: hand back the original text rather than a rebuilt equal one, so a word
    // that merely mentions a brace is not paying for a reconstruction.
    if expanded.len() == 1 && expanded[0] == atoms {
        return vec![word.to_string()];
    }
    expanded.iter().map(|a| from_atoms(a)).collect()
}

/// Brace-expand every word of a string that holds a *list* of words, rewriting it in place.
///
/// Two places in rush hold a list of words as one string and lex it themselves rather than going
/// through the parser: an alias body (`alias mk='mkdir -p {a,b}'`) and a `declare -a` array
/// literal. Brace expansion happens per word and before the lexer, so neither can get it from
/// [`crate::expand::expand_word`] — this is the same pass applied at the same boundary, one word
/// at a time.
///
/// The separators between the words are copied through untouched rather than normalised, so a
/// newline inside an alias body stays a newline.
pub fn expand_braces_in_line(text: &str) -> String {
    if !text.contains('{') {
        return text.to_string();
    }

    let atoms = to_atoms(text);
    let mut out = String::with_capacity(text.len());
    let mut word: Vec<Atom> = Vec::new();
    for atom in atoms {
        // Only an unquoted blank separates words; a quoted one is part of the word it sits in.
        if matches!(atom, Atom::Raw(c) if c.is_whitespace()) {
            push_expanded(&mut out, &word);
            word.clear();
            out.push_str(&from_atoms(std::slice::from_ref(&atom)));
        } else {
            word.push(atom);
        }
    }
    push_expanded(&mut out, &word);
    out
}

/// Append one word's brace expansion, the alternatives separated so the lexer sees several words.
fn push_expanded(out: &mut String, word: &[Atom]) {
    for (i, expanded) in expand_atoms(word).iter().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        out.push_str(&from_atoms(expanded));
    }
}

/// Split the word's text into the characters brace syntax may read and the runs it may not.
///
/// The opaque runs are the constructs whose interior belongs to a later stage: quotes, an escape,
/// a parameter expansion, a command substitution, an arithmetic expansion. Every one of them can
/// contain a `{`, `}` or `,` that is not brace syntax — `${x:-a,b}` is one word in bash, and
/// `$(f a,b)` is one too — and none of them can be recognised after the fact from a lexed word.
///
/// A `$` that starts none of those is an ordinary character: `{$v,y}z` has to leave `$v` exposed,
/// because fusing it with the `z` after the group is the entire point of running here.
fn to_atoms(text: &str) -> Vec<Atom> {
    let chars: Vec<char> = text.chars().collect();
    let mut atoms = Vec::with_capacity(chars.len());
    let mut i = 0;
    while i < chars.len() {
        let end = match chars[i] {
            // A backslash quotes exactly one character, itself included if nothing follows.
            '\\' => (i + 2).min(chars.len()),
            '\'' => closing(&chars, i + 1, '\'', false),
            '"' => closing(&chars, i + 1, '"', true),
            '`' => closing(&chars, i + 1, '`', true),
            '$' if matches!(chars.get(i + 1), Some('\'')) => closing(&chars, i + 2, '\'', true),
            // `${...}` and `$(...)`, the latter covering `$((...))` because its parentheses
            // balance. Both nest, and both may contain quotes that hide a closing delimiter.
            '$' if matches!(chars.get(i + 1), Some('{')) => nested(&chars, i + 1, '{', '}'),
            '$' if matches!(chars.get(i + 1), Some('(')) => nested(&chars, i + 1, '(', ')'),
            c => {
                atoms.push(Atom::Raw(c));
                i += 1;
                continue;
            }
        };
        atoms.push(Atom::Opaque(chars[i..end].iter().collect()));
        i = end;
    }
    atoms
}

/// Index just past the `close` that ends a run starting at `from`, or the end of the text.
///
/// An unterminated quote runs to the end on purpose: the word is a syntax error and the lexer will
/// say so, but until it does, the text inside must not be mistaken for brace syntax.
fn closing(chars: &[char], from: usize, close: char, escapes: bool) -> usize {
    let mut i = from;
    while i < chars.len() {
        if escapes && chars[i] == '\\' {
            i += 2;
            continue;
        }
        if chars[i] == close {
            return i + 1;
        }
        i += 1;
    }
    chars.len()
}

/// Index just past the delimiter closing the `open` at `from`, honouring nesting and quotes.
fn nested(chars: &[char], from: usize, open: char, close: char) -> usize {
    let mut depth = 0usize;
    let mut i = from;
    while i < chars.len() {
        match chars[i] {
            '\\' => {
                i += 2;
                continue;
            }
            '\'' => {
                i = closing(chars, i + 1, '\'', false);
                continue;
            }
            '"' => {
                i = closing(chars, i + 1, '"', true);
                continue;
            }
            c if c == open => depth += 1,
            c if c == close => {
                depth -= 1;
                if depth == 0 {
                    return i + 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    chars.len()
}

fn from_atoms(atoms: &[Atom]) -> String {
    let mut out = String::new();
    for atom in atoms {
        match atom {
            Atom::Raw(c) => out.push(*c),
            Atom::Opaque(s) => out.push_str(s),
        }
    }
    out
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
    let mut budget = WORD_LIMIT;
    expand_bounded(atoms, &mut budget).unwrap_or_else(|| vec![atoms.to_vec()])
}

/// [`expand_atoms`], with [`WORD_LIMIT`] to spend. `None` once it is gone.
fn expand_bounded(atoms: &[Atom], budget: &mut usize) -> Option<Vec<Vec<Atom>>> {
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
        let suffixes = expand_bounded(&atoms[close + 1..], budget)?;
        let mut out = Vec::new();
        for alternative in alternatives {
            // An alternative may itself contain groups: `{a,b{c,d}}` is `a bc bd`.
            for body in expand_bounded(&alternative, budget)? {
                for suffix in &suffixes {
                    *budget = budget.checked_sub(1)?;
                    let mut word = Vec::with_capacity(prefix.len() + body.len() + suffix.len());
                    word.extend_from_slice(prefix);
                    word.extend_from_slice(&body);
                    word.extend_from_slice(suffix);
                    out.push(word);
                }
            }
        }
        return Some(out);
    }
    Some(vec![atoms.to_vec()])
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

#[cfg(test)]
mod tests {
    use super::expand_braces_text as expand;

    #[test]
    fn comma_list_expands_with_prefix_and_suffix() {
        assert_eq!(expand("x{a,b}y"), vec!["xay", "xby"]);
    }

    /// The idiom the whole pass exists for: one word in, several *words* out.
    #[test]
    fn whole_word_groups_are_the_point() {
        assert_eq!(expand("build/{bin,lib}"), vec!["build/bin", "build/lib"]);
        assert_eq!(
            expand("out/{a,b}/{x,y}"),
            vec!["out/a/x", "out/a/y", "out/b/x", "out/b/y"]
        );
        assert_eq!(
            expand("file{1,2,3}.txt"),
            vec!["file1.txt", "file2.txt", "file3.txt"]
        );
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

    /// A product that would run the shell out of memory is declined, not attempted — and it is
    /// declined at *parse* time, so an `if` that never runs the line cannot be wedged by it.
    #[test]
    fn an_unbounded_product_is_declined_like_any_other_group() {
        let huge = "{1..100000}{1..100000}";
        assert_eq!(expand(huge), vec![huge]);
        // The limit is on the product; a single range that reaches it still expands.
        assert_eq!(expand("{1..100000}").len(), 100_000);
    }

    #[test]
    fn words_without_braces_are_returned_untouched() {
        assert_eq!(expand("plain"), vec!["plain"]);
    }

    /// Quoting is what decides whether a brace is syntax, and this pass has to decide it for
    /// itself because the word has not been lexed yet.
    #[test]
    fn quoting_makes_a_brace_literal() {
        assert_eq!(expand("\"{a,b}\""), vec!["\"{a,b}\""]);
        assert_eq!(expand("'{a,b}'"), vec!["'{a,b}'"]);
        assert_eq!(expand("{a\\,b}"), vec!["{a\\,b}"]);
        assert_eq!(expand("\\{a,b\\}"), vec!["\\{a,b\\}"]);
        // A quoted run is opaque, not invisible: the group around it is still a group.
        assert_eq!(expand("{\"a,b\",c}"), vec!["\"a,b\"", "c"]);
    }

    /// An expansion is one opaque run, so a comma or brace inside it is not brace syntax.
    #[test]
    fn expansions_are_opaque() {
        assert_eq!(expand("${x:-a,b}"), vec!["${x:-a,b}"]);
        assert_eq!(expand("${x:-{a,b}}"), vec!["${x:-{a,b}}"]);
        assert_eq!(expand("$(f a,b)"), vec!["$(f a,b)"]);
        assert_eq!(expand("$((1,2))"), vec!["$((1,2))"]);
        assert_eq!(expand("`f a,b`"), vec!["`f a,b`"]);
        assert_eq!(expand("$'a,b'"), vec!["$'a,b'"]);
        // …and a group *containing* one still expands, carrying it through untouched.
        assert_eq!(expand("{$(f a,b),y}"), vec!["$(f a,b)", "y"]);
    }

    /// The reason this pass runs before the lexer: a group boundary can land inside a name.
    #[test]
    fn a_group_boundary_fuses_into_the_text_around_it() {
        assert_eq!(expand("{$v,y}z"), vec!["$vz", "yz"]);
        assert_eq!(expand("pre{$v,y}post"), vec!["pre$vpost", "preypost"]);
        assert_eq!(expand("$v{a,b}"), vec!["$va", "$vb"]);
        // `${v}` closes the name itself, so there is nothing for the group to fuse into.
        assert_eq!(expand("${v}{a,b}"), vec!["${v}a", "${v}b"]);
    }

    /// An unterminated quote is a syntax error the lexer will report; until it does, the text
    /// inside must not be read as brace syntax.
    #[test]
    fn an_unterminated_quote_swallows_the_rest_of_the_word() {
        assert_eq!(expand("'{a,b}"), vec!["'{a,b}"]);
        assert_eq!(expand("x\"{a,b}"), vec!["x\"{a,b}"]);
    }

    mod in_line {
        use super::super::expand_braces_in_line as line;

        #[test]
        fn each_word_expands_on_its_own() {
            assert_eq!(line("mkdir -p {a,b}"), "mkdir -p a b");
            assert_eq!(line("cp {a,b} {c,d}"), "cp a b c d");
        }

        /// The separators are the caller's, not this pass's: an alias body may be several lines.
        #[test]
        fn separators_survive_verbatim() {
            assert_eq!(line("echo\t{a,b}\nls"), "echo\ta b\nls");
            assert_eq!(line("echo  a"), "echo  a");
        }

        #[test]
        fn a_line_without_a_group_is_returned_unchanged() {
            assert_eq!(line("grep --color \"a b\""), "grep --color \"a b\"");
            assert_eq!(line("echo a{b}c"), "echo a{b}c");
        }

        /// A quoted blank does not separate words, so the group around it stays one word.
        #[test]
        fn a_quoted_blank_does_not_split_a_word() {
            assert_eq!(line("echo {\"a b\",c}"), "echo \"a b\" c");
        }
    }
}

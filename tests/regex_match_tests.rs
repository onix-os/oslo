//! `[[ =~ ]]` with a regex that was built rather than written out.
//!
//! # What this exists to catch
//!
//! ```text
//! R="a|b"; [[ a =~ ^($R)$ ]]      bash: match       oslo: no match
//! R="a|b"; [[ a =~ (${R}) ]]      bash: match       oslo: invalid regular expression `(${R})'
//! ```
//!
//! The variable never expanded. `crate::syntax::brush_adapter::words::convert_words_from_str`
//! re-lexes an operand's raw text with the *shell's* lexer; `(` is a shell operator there, so the
//! word "turns out not to be a plain word" and falls back to `Word::from_literal` — a part nothing
//! ever expands. A bare `$R` worked, because a bare `$R` lexes cleanly. Putting it in a group
//! broke it, and putting a regex in a group is what people do.
//!
//! **It gave the wrong answer rather than refusing**, which is the failure mode `docs/known-gaps.md`
//! opens by saying oslo does not have: every gap there fails loudly. A condition that quietly never
//! matches is how a check that cannot fail gets written, and this repo had one — `scripts/check-
//! readme.sh` builds a regex in a variable, matched nothing under oslo, and passed while the docs
//! it was checking were wrong.
//!
//! Every expectation below was run against `bash` on the same line.

mod common;

use common::run;

/// A variable in the regex expands, and its metacharacters are the regex's own.
#[test]
fn an_unquoted_expansion_carries_regex_metacharacters() {
    for line in [
        r#"R="a|b"; [[ "a" =~ ^($R)$ ]] && echo MATCH"#,
        r#"R="a|b"; [[ "a" =~ ($R) ]] && echo MATCH"#,
        r#"R="a|b"; [[ "a" =~ (${R}) ]] && echo MATCH"#,
        r#"R="a|b"; [[ "a" =~ ^$R$ ]] && echo MATCH"#,
        r#"R="a|b"; [[ "za" =~ (z)($R) ]] && echo MATCH"#,
        // The shape the repo's own script uses: an alternation of paths, anchored.
        r#"R="docs|crates"; [[ "crates/oslo-shell" =~ ^($R)/ ]] && echo MATCH"#,
    ] {
        let r = run(line);
        assert_eq!(r.out(), "MATCH", "`{line}` did not match: {}", r.stderr);
    }
}

/// **A quoted operand stays literal**, which is the other half of bash's rule and must not be lost
/// while fixing the first half.
#[test]
fn a_quoted_expansion_is_matched_literally() {
    let r = run(r#"R="a|b"; [[ "a|b" =~ ^"$R"$ ]] && echo MATCH"#);
    assert_eq!(r.out(), "MATCH", "stderr: {}", r.stderr);

    // And therefore does *not* match either alternative on its own.
    let r = run(r#"R="a|b"; [[ "a" =~ ^"$R"$ ]] && echo MATCH || echo NOMATCH"#);
    assert_eq!(r.out(), "NOMATCH", "stderr: {}", r.stderr);
}

/// A regex written out in full still behaves, which is what made the bug hard to see.
#[test]
fn a_literal_regex_is_unaffected() {
    for (line, expected) in [
        (
            r#"[[ "ab" =~ ^(a|c)b$ ]] && echo MATCH || echo NOMATCH"#,
            "MATCH",
        ),
        (
            r#"[[ "xb" =~ ^(a|c)b$ ]] && echo MATCH || echo NOMATCH"#,
            "NOMATCH",
        ),
        (
            r#"[[ "abc" =~ a.c ]] && echo MATCH || echo NOMATCH"#,
            "MATCH",
        ),
        // Quoting turns the metacharacters off, so the dot is a dot.
        (
            r#"[[ "abc" =~ "a.c" ]] && echo MATCH || echo NOMATCH"#,
            "NOMATCH",
        ),
    ] {
        let r = run(line);
        assert_eq!(r.out(), expected, "`{line}`: {}", r.stderr);
    }
}

/// The groups reach `BASH_REMATCH`, which is what a script reads the match back out of.
#[test]
fn the_groups_of_an_expanded_regex_are_captured() {
    let r = run(r#"R="[0-9]+"; [[ "v42x" =~ v($R)x ]]; echo "${BASH_REMATCH[1]}""#);
    assert_eq!(r.out(), "42", "stderr: {}", r.stderr);
}

/// An expansion that is *not* a regex still works — the fix must not make ordinary text special.
#[test]
fn an_ordinary_expansion_still_matches_itself() {
    let r = run(r#"NAME="oslo"; [[ "oslo-shell" =~ ^($NAME)- ]] && echo MATCH"#);
    assert_eq!(r.out(), "MATCH", "stderr: {}", r.stderr);

    // A value containing regex-special characters that the script means literally is what quoting
    // is for, and it still is.
    let r = run(r#"V="a.c"; [[ "abc" =~ ^"$V"$ ]] && echo MATCH || echo NOMATCH"#);
    assert_eq!(r.out(), "NOMATCH", "stderr: {}", r.stderr);
}

//! `=~` against bash, expression by expression.
//!
//! Every expected value here was taken from `bash -c`, including the ones that look surprising
//! (`[[ abc =~ "a.c" ]]` is *false*; `[[ abc =~ b ]]` is *true*, because the match is a search and
//! not an anchored comparison). The table form exists because the operator's failure mode is a
//! plausible wrong answer: an implementation that anchored the match, or that treated a quoted
//! operand as a pattern, would still exit 0 or 1 and look healthy.

use super::super::{builtin_extended_test, builtin_test};
use super::{REGEX_LITERAL_OP, REGEX_OP};
use crate::env::scope::Environment;

/// Run `[[ subject op operand ]]` and return `(status, env)`.
fn run(subject: &str, op: &str, operand: &str) -> (i32, Environment) {
    let mut env = Environment::new();
    let argv: Vec<String> = ["[[", subject, op, operand, "]]"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let status = builtin_extended_test(&mut env, &argv).expect("[[ must not unwind");
    (status, env)
}

fn status(subject: &str, op: &str, operand: &str) -> i32 {
    run(subject, op, operand).0
}

/// `(subject, pattern, status)` for the unquoted — pattern — form.
const PATTERNS: &[(&str, &str, i32)] = &[
    // The validation idiom the whole finding is about.
    ("42", "^[0-9]+$", 0),
    ("42x", "^[0-9]+$", 1),
    ("", "^[0-9]+$", 1),
    ("-1", "^-?[0-9]+$", 0),
    // A match is a *search*: unanchored, anywhere in the subject.
    ("abc", "b", 0),
    ("abc", "^b", 1),
    ("abc", "c$", 0),
    ("abc", "^abc$", 0),
    // Metacharacters mean what ERE says they mean.
    ("abc", "a.c", 0),
    ("ac", "a.c", 1),
    ("aaa", "a{2,3}", 0),
    ("a", "a{2,3}", 1),
    ("cat", "^(cat|dog)$", 0),
    ("cow", "^(cat|dog)$", 1),
    // POSIX bracket expressions, which a glob engine would also accept but read differently.
    ("abc", "[[:alpha:]]+", 0),
    ("123", "[[:alpha:]]+", 1),
    ("a1", "^[[:alpha:]][[:digit:]]$", 0),
    // `*` is a repetition operator here, not a glob wildcard: `a*` matches the empty string, so
    // every subject matches. A glob-backed implementation would answer 1 for "xyz".
    ("xyz", "a*", 0),
    // POSIX `.` matches a newline; the regex crate's default does not, so this pins the override.
    ("a\nb", "a.b", 0),
    // Case is significant.
    ("ABC", "abc", 1),
    // An empty pattern matches everything, as bash's does.
    ("anything", "", 0),
];

/// `(subject, literal, status)` for the quoted form: the operand is ordinary text.
const LITERALS: &[(&str, &str, i32)] = &[
    ("abc", "a.c", 1),
    ("a.c", "a.c", 0),
    // Still a search, so a literal substring matches.
    ("abc", "b", 0),
    ("abc", "^abc$", 1),
    // Text that would not even compile as a regex is fine as a literal.
    ("abc", "(", 1),
    ("a(c", "(", 0),
    ("a*b", "*", 0),
];

#[test]
fn unquoted_operand_is_an_extended_regular_expression() {
    for (subject, pattern, expected) in PATTERNS {
        assert_eq!(
            status(subject, REGEX_OP, pattern),
            *expected,
            "[[ {:?} =~ {:?} ]]",
            subject,
            pattern
        );
    }
}

#[test]
fn quoted_operand_is_matched_literally() {
    for (subject, literal, expected) in LITERALS {
        assert_eq!(
            status(subject, REGEX_LITERAL_OP, literal),
            *expected,
            "[[ {:?} =~ \"{}\" ]]",
            subject,
            literal
        );
    }
}

#[test]
fn a_match_publishes_the_whole_match_and_every_group() {
    let (status, env) = run("abc123", REGEX_OP, "([a-z]+)([0-9]+)");
    assert_eq!(status, 0);

    let m = env.get_array("BASH_REMATCH").expect("BASH_REMATCH is set");
    assert_eq!(m.values().collect::<Vec<_>>(), ["abc123", "abc", "123"]);
}

#[test]
fn a_group_that_did_not_participate_is_an_empty_element_not_a_hole() {
    // bash reports 4 elements for this, with element 2 empty. Dropping the element instead would
    // shift `${BASH_REMATCH[3]}` onto the wrong group.
    let (status, env) = run("ab", REGEX_OP, "(a)(x)?(b)");
    assert_eq!(status, 0);

    let m = env.get_array("BASH_REMATCH").expect("BASH_REMATCH is set");
    assert_eq!(m.len(), 4);
    assert_eq!(m.values().collect::<Vec<_>>(), ["ab", "a", "", "b"]);
}

#[test]
fn a_failed_match_clears_the_previous_captures() {
    let mut env = Environment::new();
    let call = |env: &mut Environment, pattern: &str| {
        let argv: Vec<String> = ["[[", "abc", REGEX_OP, pattern, "]]"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        builtin_extended_test(env, &argv).expect("[[ must not unwind")
    };

    assert_eq!(call(&mut env, "(b)"), 0);
    assert_eq!(env.get_array("BASH_REMATCH").map(|m| m.len()), Some(2));

    // Leaving the old groups in place is the dangerous failure: `[[ $x =~ $p ]] ; use
    // ${BASH_REMATCH[1]}` would read the previous subject's capture.
    assert_eq!(call(&mut env, "zzz"), 1);
    assert_eq!(env.get_array("BASH_REMATCH").map(|m| m.len()), Some(0));
}

#[test]
fn the_literal_form_also_publishes_the_match() {
    let (status, env) = run("hello world", REGEX_LITERAL_OP, "o w");
    assert_eq!(status, 0);

    let m = env.get_array("BASH_REMATCH").expect("BASH_REMATCH is set");
    assert_eq!(m.values().collect::<Vec<_>>(), ["o w"]);
}

#[test]
fn an_invalid_pattern_is_a_syntax_error_not_a_negative_answer() {
    // bash: `[[: invalid regular expression`, exit 2. Answering 1 would tell a validation guard
    // that the input failed the check, which is a different and wrong claim.
    for bad in ["(", "a{2,1}", "[z-a]", "*"] {
        assert_eq!(status("abc", REGEX_OP, bad), 2, "pattern {:?}", bad);
    }
}

#[test]
fn an_invalid_pattern_leaves_the_previous_captures_alone() {
    let mut env = Environment::new();
    let call = |env: &mut Environment, pattern: &str| {
        let argv: Vec<String> = ["[[", "abc", REGEX_OP, pattern, "]]"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        builtin_extended_test(env, &argv).expect("[[ must not unwind")
    };

    assert_eq!(call(&mut env, "(b)"), 0);
    assert_eq!(call(&mut env, "("), 2);
    assert_eq!(
        env.get_array("BASH_REMATCH")
            .and_then(|m| m.get(1))
            .map(str::to_string),
        Some("b".to_string()),
        "a rejected pattern must not disturb BASH_REMATCH, as in bash"
    );
}

#[test]
fn regex_matching_is_extended_test_only() {
    // `[ a =~ a ]` is `=~: binary operator expected`, exit 2, in bash. `=~` deliberately stays out
    // of `is_binary_op` so the POSIX grammar refuses it rather than quietly matching.
    let mut env = Environment::new();
    let argv: Vec<String> = ["[", "abc", "=~", "b", "]"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    assert_eq!(builtin_test(&mut env, &argv).expect("[ must not unwind"), 2);

    // And the internal literal spelling is not reachable from `test` either.
    let argv: Vec<String> = ["[", "abc", REGEX_LITERAL_OP, "b", "]"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    assert_eq!(builtin_test(&mut env, &argv).expect("[ must not unwind"), 2);
}

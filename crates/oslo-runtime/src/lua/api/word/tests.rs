//! Both bindings are pure functions of a string, so they need no VM — [`probe`] calls them
//! directly. That is also the property being asserted: a test that needed an interpreter would
//! mean the binding had grown a reach into the shell.

use super::super::util::probe;
use super::*;

fn table() -> Value {
    build()
}

/// `Value` has no `PartialEq`, so a boolean answer is read through `truthy` — which is also the
/// question a caller asks, since `if oslo.word.matches(...)` is the whole point.
fn matched(name: &str, args: Vec<Value>) -> bool {
    call(name, args).truthy()
}

fn call(name: &str, args: Vec<Value>) -> Value {
    let built = table();
    let Value::Table(t) = &built else {
        panic!("not a table")
    };
    let f = t.borrow().get_str(name);
    probe::first(&f, args)
}

fn strings(value: &Value) -> Vec<String> {
    let Value::Table(t) = value else {
        panic!("not a list: {}", value.type_name())
    };
    let t = t.borrow();
    (1..)
        .map_while(|i| match t.get(&Value::int(i)) {
            Value::Str(s) => Some(s.to_string()),
            _ => None,
        })
        .collect()
}

/// The common case, and the reason the result is always a list: a word with no group is a list of
/// one, so a caller can iterate without asking whether anything expanded.
#[test]
fn a_word_with_no_group_is_a_list_of_itself() {
    let out = call("braces", vec![Value::str("plain.rs")]);
    assert_eq!(strings(&out), vec!["plain.rs"]);
}

#[test]
fn a_group_gives_one_word_per_alternative() {
    let out = call("braces", vec![Value::str("src/{a,b}.rs")]);
    assert_eq!(strings(&out), vec!["src/a.rs", "src/b.rs"]);
}

/// A range is the half of brace expansion nobody writes correctly by hand.
#[test]
fn a_numeric_range_expands() {
    let out = call("braces", vec![Value::str("f{1..3}")]);
    assert_eq!(strings(&out), vec!["f1", "f2", "f3"]);
}

#[test]
fn a_star_matches_and_a_mismatch_does_not() {
    assert!(matched(
        "matches",
        vec![Value::str("main.rs"), Value::str("*.rs")]
    ));
    assert!(!matched(
        "matches",
        vec![Value::str("main.go"), Value::str("*.rs")]
    ));
}

/// **`?` is one character and `.` is not a metacharacter.** This is the difference between a shell
/// glob and a Lua pattern, and it is the reason this binding exists rather than a `string.find`
/// with `*` rewritten to `.*` — that rewrite gets `a.txt` against `a?txt` wrong in both directions.
#[test]
fn the_pattern_language_is_the_shells_not_luas() {
    assert!(matched(
        "matches",
        vec![Value::str("a.txt"), Value::str("a?txt")]
    ));
    assert!(
        !matched("matches", vec![Value::str("axtxt"), Value::str("a.txt")]),
        "`.` matched a letter, so this is a Lua pattern rather than a glob"
    );
}

/// A slash is an ordinary character at this layer, and a leading dot is not special. Those rules
/// belong to pathname expansion, which is a different thing; pinned so nobody "fixes" it.
#[test]
fn a_slash_is_ordinary_here() {
    assert!(matched(
        "matches",
        vec![Value::str("a/b"), Value::str("a*b")]
    ));
    assert!(matched(
        "matches",
        vec![Value::str(".hidden"), Value::str("*")]
    ));
}

#[test]
fn a_character_class_works() {
    assert!(matched(
        "matches",
        vec![Value::str("a1"), Value::str("a[0-9]")]
    ));
    assert!(!matched(
        "matches",
        vec![Value::str("ab"), Value::str("a[0-9]")]
    ));
}

/// The cache is keyed on the pattern, so the same pattern against many subjects compiles once —
/// which is the whole reason it is there. Asserted through behaviour: repeated calls agree.
#[test]
fn the_same_pattern_answers_consistently_when_reused() {
    let built = table();
    let Value::Table(t) = &built else {
        panic!("not a table")
    };
    let f = t.borrow().get_str("matches");
    for (subject, want) in [("one.rs", true), ("two.go", false), ("three.rs", true)] {
        assert_eq!(
            probe::first(&f, vec![Value::str(subject), Value::str("*.rs")]).truthy(),
            want,
            "{subject}"
        );
    }
}

/// A missing or non-string argument is a caller mistake, so it raises rather than answering nil —
/// the rule `api/util.rs` states for every binding.
#[test]
fn a_bad_argument_raises() {
    let built = table();
    let Value::Table(t) = &built else {
        panic!("not a table")
    };
    let braces = t.borrow().get_str("braces");
    assert!(probe::call(&braces, vec![]).is_err(), "no argument");
    assert!(
        probe::call(&braces, vec![Value::Bool(true)]).is_err(),
        "a boolean is not a word"
    );
    let matches = t.borrow().get_str("matches");
    assert!(
        probe::call(&matches, vec![Value::str("a")]).is_err(),
        "a pattern is required"
    );
}

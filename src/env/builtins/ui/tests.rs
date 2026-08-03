//! `ui`'s command line: what each widget accepts, and what a script sees when there is no
//! terminal to ask on.
//!
//! The widgets themselves are tested in `interactive::ask`. What is testable here is the contract
//! a script is written against — the statuses — because under `cargo test` stdin is never a
//! terminal, which is exactly the headless path scripts hit in CI.

use super::*;

fn run(args: &[&str]) -> i32 {
    let mut env = Environment::new();
    let owned: Vec<String> = std::iter::once("ui".to_string())
        .chain(args.iter().map(|s| s.to_string()))
        .collect();
    builtin_ui(&mut env, &owned).expect("ui never fails fatally")
}

/// No widget named is a usage error, not a crash and not a silent success.
#[test]
fn a_missing_or_unknown_widget_is_a_usage_error() {
    assert_eq!(run(&[]), 2);
    assert_eq!(run(&["nonesuch"]), 2);
    assert_eq!(run(&["--help"]), 0);
}

/// `--value` is the answer when there is nobody to ask, which is what lets a script using `ui`
/// still run in CI.
#[test]
fn input_answers_with_its_default_when_there_is_no_terminal() {
    assert_eq!(run(&["input", "--value", "fallback"]), 0);
    // Without one there is no answer, and that is status 2 rather than an empty success.
    assert_eq!(run(&["input"]), 2);
}

/// `confirm`'s answer *is* its status: 0 for yes, 1 for no, so `ui confirm && …` works.
#[test]
fn confirm_answers_through_its_status() {
    assert_eq!(run(&["confirm", "sure?"]), 1, "defaults to no");
    assert_eq!(run(&["confirm", "--default", "sure?"]), 0);
}

/// An empty list is not a question. Status 1 keeps `x=$(… | ui choose) || exit` correct when the
/// pipeline produced no lines.
#[test]
fn choosing_from_nothing_is_a_cancel() {
    assert_eq!(run(&["choose"]), 1);
}

/// With items but no terminal, `choose` refuses rather than picking one on the script's behalf.
#[test]
fn choose_refuses_without_a_terminal() {
    assert_eq!(run(&["choose", "alpha", "beta"]), 2);
    assert_eq!(run(&["filter", "alpha", "beta"]), 2);
}

/// `style` asks nothing, so it works headless and answers 0.
#[test]
fn style_needs_no_terminal() {
    assert_eq!(run(&["style", "hello"]), 0);
    assert_eq!(run(&["style", "--border", "rounded", "hi"]), 0);
    assert_eq!(
        run(&["style", "--border", "fancy", "hi"]),
        2,
        "unknown border"
    );
}

/// An unknown option is refused rather than being taken as an item — otherwise `ui choose --oops`
/// would silently offer `--oops` as something to pick.
#[test]
fn unknown_options_are_refused() {
    assert_eq!(run(&["input", "--nope"]), 2);
    assert_eq!(run(&["choose", "--nope"]), 2);
    assert_eq!(run(&["confirm", "--nope"]), 2);
    assert_eq!(run(&["style", "--nope"]), 2);
}

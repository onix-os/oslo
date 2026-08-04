//! `ui`'s command line: what each widget accepts, and what a script sees when there is no
//! terminal to ask on.
//!
//! The widgets themselves are tested in `interactive::ask`. What is testable here is the contract
//! a script is written against — the statuses — because under `cargo test` there is no terminal,
//! which is exactly the headless path scripts hit in CI.
//!
//! **Nothing here may run a widget that falls back to stdin.** `ui choose` with no operands reads
//! stdin for its items, and "not a terminal" is not the same as "at end of input": a test harness
//! hands the process a *pipe*, which blocks until somebody closes the far end, and nobody does.
//! That is a hang of the whole suite — no failure, no output, just a run that never finishes —
//! and it depends on how the runner was launched, so it reproduces on one machine and not the
//! next. It cost two full verify runs before it was pinned down.
//!
//! The status mapping those calls were reaching for is asserted directly on [`report`] instead,
//! and the widget behaviour behind it in `interactive::ask::choose`.

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

/// An empty list is not a question, and a cancel is status 1 — which is what keeps
/// `x=$(… | ui choose) || exit` correct when the pipeline produced no lines.
///
/// Asserted on `report` rather than by running `ui choose` with no operands: that would read
/// stdin, and see the module comment for why doing so here hangs the suite. That an empty list
/// cancels at all is `interactive::ask::choose`'s `an_empty_list_cancels`.
#[test]
fn a_cancelled_answer_is_status_one() {
    assert_eq!(report(Answer::Cancelled), 1);
    assert_eq!(report(Answer::NoTerminal), 2, "and nobody to ask is 2");
    assert_eq!(
        report(Answer::Given(vec!["picked".to_string()])),
        0,
        "an answer is a success"
    );
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

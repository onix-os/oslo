use super::*;

fn run(words: &[&str]) -> i32 {
    let args: Vec<String> = std::iter::once("messages")
        .chain(words.iter().copied())
        .map(String::from)
        .collect();
    let mut env = Environment::new();
    builtin_messages(&mut env, &args).expect("messages never fails")
}

#[test]
fn an_option_it_does_not_know_is_a_usage_error() {
    assert_eq!(run(&["--nonsense"]), 2);
    assert_eq!(run(&["-n"]), 2, "-n wants a number");
    assert_eq!(run(&["-n", "x"]), 2);
}

/// **An empty session is not a failure.** `messages --errors` in a script asks whether anything went
/// wrong; the status must not be the answer, or every clean session looks broken.
#[test]
fn saying_nothing_still_succeeds() {
    messages::clear();
    assert_eq!(run(&[]), 0);
    assert_eq!(run(&["--errors"]), 0);
    assert_eq!(run(&["-n", "5"]), 0);
    assert_eq!(run(&["nosuchsource"]), 0);
}

#[test]
fn clear_empties_the_buffer() {
    messages::say(Level::Warn, "test", "something");
    assert_eq!(run(&["--clear"]), 0);
    assert!(messages::all().is_empty());
}

#[test]
fn help_is_not_an_error() {
    assert_eq!(run(&["--help"]), 0);
    assert_eq!(run(&["-h"]), 0);
}

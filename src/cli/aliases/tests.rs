use super::*;

#[test]
fn a_subcommand_nobody_has_is_a_usage_error() {
    assert_eq!(run(&["nonsense".to_string()]), 2);
}

#[test]
fn help_is_not_an_error() {
    for word in ["--help", "-h", "help"] {
        assert_eq!(run(&[word.to_string()]), 0, "{word}");
    }
}

/// Bare `oslo aliases` prints the overview and is a usage error, like `history` and `plugin`.
#[test]
fn saying_nothing_is_a_usage_error() {
    assert_eq!(run(&[]), 2);
}

fn words(args: &[&str]) -> Vec<String> {
    args.iter().map(|a| a.to_string()).collect()
}

#[test]
fn a_flag_names_the_kind_and_the_rest_are_words() {
    let asked = parse(&words(&["--abbrev", "gs", "git status"])).expect("parses");
    assert_eq!(asked.kind, Kind::Abbrev);
    assert_eq!(asked.words, ["gs", "git status"]);

    assert_eq!(parse(&words(&["--func", "x"])).unwrap().kind, Kind::Func);
    assert_eq!(
        parse(&words(&["--script", "x"])).unwrap().kind,
        Kind::Script
    );
    assert_eq!(
        parse(&words(&["x"])).unwrap().kind,
        Kind::Alias,
        "the default"
    );
}

#[test]
fn two_kinds_at_once_is_a_mistake_rather_than_the_last_one_winning() {
    let problem = parse(&words(&["--func", "--script", "x"])).unwrap_err();
    assert!(problem.contains("one kind"), "{problem}");
}

/// **A body is arbitrary text and often starts with a dash.** `oslo aliases add ll '-la'` has to
/// store `-la`, not report an unknown option.
#[test]
fn a_dash_after_the_name_is_a_body_not_an_option() {
    let asked = parse(&words(&["ll", "--color=auto"])).expect("parses");
    assert_eq!(asked.words, ["ll", "--color=auto"]);

    // Before any word, it is an option, and an unknown one is still a mistake.
    assert!(parse(&words(&["--nonsense", "x"])).is_err());
}

#[test]
fn the_switches_are_read() {
    assert!(parse(&words(&["--edit", "x"])).unwrap().edit);
    assert!(parse(&words(&["--plain"])).unwrap().plain);
    assert!(parse(&words(&["--replace"])).unwrap().replace);
}

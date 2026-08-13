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

/// Bare `oslo macros` prints the overview and is a usage error, like `history` and `plugin`.
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
    assert_eq!(asked.kind, Some(Kind::Abbrev));
    assert_eq!(asked.words, ["gs", "git status"]);

    for (flag, kind) in [
        ("--alias", Kind::Alias),
        ("--func", Kind::Func),
        ("--script", Kind::Script),
    ] {
        assert_eq!(parse(&words(&[flag, "x"])).unwrap().kind, Some(kind));
    }
}

/// **There is no default kind.** `add gs 'git status'` meaning an alias because alias came first
/// in an enum is exactly the trap this avoids.
#[test]
fn a_kind_has_to_be_said() {
    assert_eq!(parse(&words(&["x"])).unwrap().kind, None);
    let problem = parse(&words(&["x"])).unwrap().kind().unwrap_err();
    assert!(problem.contains("--alias"), "{problem}");
    assert_eq!(run(&words(&["add", "gs", "git status"])), 2);
}

#[test]
fn two_kinds_at_once_is_a_mistake_rather_than_the_last_one_winning() {
    let problem = parse(&words(&["--func", "--script", "x"])).unwrap_err();
    assert!(problem.contains("one kind"), "{problem}");
    // The same flag twice is not two kinds; it is somebody being emphatic.
    assert!(parse(&words(&["--func", "--func", "x"])).is_ok());
}

#[test]
fn tags_are_collected_and_checked() {
    let asked = parse(&words(&["--alias", "gs", "--tag", "git", "-t", "system"])).expect("parses");
    assert_eq!(asked.tags, ["git", "system"]);
    assert_eq!(asked.words, ["gs"]);

    assert!(parse(&words(&["--tag"])).is_err(), "a tag with no tag");
    assert!(parse(&words(&["--tag", "two words"])).is_err());
}

/// A body on the command line is refused for the two kinds that cannot fit on one.
#[test]
fn a_function_may_not_be_written_inline() {
    assert_eq!(run(&words(&["add", "--func", "f", "echo hi"])), 2);
    assert_eq!(run(&words(&["add", "--script", "s", "echo hi"])), 2);
}

/// **A body is arbitrary text and often starts with a dash.** `oslo macros add ll '-la'` has to
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

#[test]
fn edit_needs_a_name_and_reports_one_it_does_not_have() {
    assert_eq!(run(&words(&["edit"])), 2, "no name is a usage error");
    assert_eq!(
        run(&words(&["edit", "no-such-macro-anywhere"])),
        1,
        "a name nothing is stored under is an error, not a new macro"
    );
}

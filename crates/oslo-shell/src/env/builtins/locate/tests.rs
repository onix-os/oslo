use super::{Options, line, parse_options};
use crate::env::builtins::control::ways;
use crate::env::scope::Environment;

fn args(words: &[&str]) -> Vec<String> {
    words.iter().map(|s| s.to_string()).collect()
}

/// The one line a caller pipes into something else: a path, alone, with nothing around it.
#[test]
fn a_program_answers_with_a_bare_path() {
    let env = Environment::new();
    let kinds = ways(&env, "sh", false);
    let path = kinds.first().expect("sh is somewhere");
    assert_eq!(
        line("sh", path),
        path.path().expect("a file").display().to_string()
    );
}

/// **A builtin has no file, and saying so is the whole reason this is a builtin.** `/usr/bin/which
/// cd` answers nothing at all.
#[test]
fn a_builtin_says_what_it_is() {
    let mut env = Environment::new();
    crate::env::builtins::register_default_builtins(&mut env);
    let kinds = ways(&env, "cd", false);
    assert_eq!(
        line("cd", kinds.first().expect("cd is a builtin")),
        "cd: shell built-in command"
    );
}

/// An alias reports its body, because "it is an alias" without saying to what is half an answer.
#[test]
fn an_alias_reports_what_it_expands_to() {
    let mut env = Environment::new();
    env.set_alias("ll", "ls -alF");
    let kinds = ways(&env, "ll", false);
    assert_eq!(
        line("ll", kinds.first().expect("an alias")),
        "ll: aliased to ls -alF"
    );
}

/// `which if` answers, as zsh does — a reserved word is a thing the line can begin with.
#[test]
fn a_reserved_word_is_reported() {
    let env = Environment::new();
    let kinds = ways(&env, "if", false);
    assert_eq!(
        line("if", kinds.first().expect("a keyword")),
        "if: shell reserved word"
    );
}

/// `--skip-alias` is how a script asks for the program's behaviour, and it has to mean it.
#[test]
fn skip_alias_asks_only_the_path() {
    let given = args(&["--skip-alias", "ll"]);
    let (opts, names) = parse_options(&given).expect("parsed");
    assert!(opts.skip_shell);
    assert_eq!(names, ["ll"]);

    let mut env = Environment::new();
    env.set_alias("sh", "sh --posix");
    // The flag asks for the `$PATH` answer, and gets a file rather than the alias.
    let skipped = super::ways_for(&env, "sh", &opts);
    assert!(
        skipped.iter().all(|kind| kind.path().is_some()),
        "the alias survived --skip-alias"
    );
    assert_eq!(
        super::ways_for(&env, "sh", &Options::default())
            .first()
            .and_then(|kind| kind.alias_body()),
        Some("sh --posix"),
        "without the flag the alias is the answer"
    );
}

/// Clustered short options, and the operands that follow them.
#[test]
fn the_short_options_cluster() {
    let given = args(&["-as", "ls", "cd"]);
    let (opts, names) = parse_options(&given).expect("parsed");
    assert!(opts.all && opts.silent);
    assert_eq!(names, ["ls", "cd"]);
}

/// An option nobody implements is a usage error, not a name to look up.
#[test]
fn an_unknown_option_is_refused() {
    assert_eq!(parse_options(&args(&["-Z", "ls"])).unwrap_err(), 2);
    assert_eq!(parse_options(&args(&["--nope"])).unwrap_err(), 2);
}

/// `--` ends the options, so a file really called `-a` can be looked up.
#[test]
fn a_double_dash_ends_the_options() {
    let given = args(&["--", "-a"]);
    let (opts, names) = parse_options(&given).expect("parsed");
    assert!(!opts.all);
    assert_eq!(names, ["-a"]);
}

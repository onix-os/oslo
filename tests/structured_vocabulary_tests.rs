//! **A word that runs must not read as a mistake.**
//!
//! The prompt asks four sources what a command word is — builtin, alias, function, `$PATH` — and a
//! structured verb is none of them. So `ls | where 'size > 1024'` was painted with the
//! command-not-found colour on both `where` and, once a config registered one, on its own tools;
//! Tab offered nothing for either; and the correction ghost stood ready to "fix" `where` into
//! whatever `$PATH` happened to have nearest.
//!
//! The vocabulary is registered by the shell and read by the interface, which is why it lives in
//! `oslo-base` — the two crates cannot see each other.

mod common;

use oslo::env::Environment;
use oslo::ui::OsloHelper;
use std::sync::{Arc, Mutex};

/// Register the shipped vocabulary, as startup does.
fn vocabulary() {
    oslo::data::tools::register_all();
}

fn helper() -> OsloHelper {
    let mut h = OsloHelper::new(Arc::new(Mutex::new(Environment::new())));
    h.set_menu(false);
    h
}

/// Every shipped verb is a name the shell can run, so every layer has to know it.
#[test]
fn a_structured_verb_is_a_name_the_prompt_knows() {
    vocabulary();
    for verb in [
        "where", "cols", "sort-by", "group-by", "to", "lines", "from",
    ] {
        assert!(
            oslo_base::vocab::contains(verb),
            "{verb} is a name the shell runs"
        );
    }
}

/// **Tab reaches them.** `whe<Tab>` found nothing at all, because no source enumerated them.
#[test]
fn a_structured_verb_is_offered_by_tab() {
    vocabulary();
    let h = helper();

    let (_, cands) = h.candidates("ls | whe", 8);
    let shown: Vec<&str> = cands.iter().map(|c| c.display.as_str()).collect();
    assert!(shown.contains(&"where"), "{shown:?}");

    // With its own badge, so the dropdown says what kind of thing it is.
    let verb = cands.iter().find(|c| c.display == "where").expect("where");
    assert_eq!(verb.kind.as_deref(), Some("verb"));
}

/// **It is not a typo.** The correction ghost is drawn from `$PATH`, which has never heard of any
/// of these — so a perfectly good verb was offered a "did you mean".
#[cfg(feature = "vista")]
#[test]
fn a_structured_verb_is_never_corrected() {
    vocabulary();
    let h = helper();
    assert_eq!(h.repair("ls | where"), None);
    assert_eq!(h.repair("where"), None);
}

/// A half-typed verb is unfinished rather than wrong, the same rule a half-typed command follows.
#[test]
fn a_half_typed_verb_is_unfinished() {
    vocabulary();
    assert!(oslo_base::vocab::has_prefix("grou"));
    assert!(oslo_base::vocab::has_prefix("sort-"));
    assert!(!oslo_base::vocab::has_prefix("zzz"));
}

/// **An autoloaded function is a command that runs**, so the prompt must not call it a typo.
///
/// `~/.config/oslo/functions/NAME.sh` defines `NAME`, found after `$PATH` fails — which is exactly
/// why no source the prompt asked could see it: not a builtin, not an alias, not yet a function,
/// and not on `$PATH`. It painted red, completed to nothing, and stood ready to be "corrected".
#[test]
fn an_autoloaded_function_is_a_name_the_prompt_knows() {
    let home = tempfile::tempdir().unwrap();
    let functions = home.path().join("oslo/functions");
    std::fs::create_dir_all(&functions).unwrap();
    std::fs::write(
        functions.join("deploy_thing.sh"),
        b"deploy_thing() { :; }\n",
    )
    .unwrap();

    let mut env = Environment::new();
    env.set_var("XDG_CONFIG_HOME", home.path().to_str().unwrap(), false);
    oslo::names::refresh(&env);

    assert!(
        oslo_base::vocab::contains("deploy_thing"),
        "an autoloaded function runs, so the prompt has to know it"
    );
    assert_eq!(oslo_base::vocab::kind_of("deploy_thing"), Some("function"));
    assert!(
        oslo_base::vocab::has_prefix("deploy_"),
        "half-typed is unfinished, not wrong"
    );

    // **Replaced wholesale, so a name that goes away stops being known.** Merging would only add.
    std::fs::remove_file(functions.join("deploy_thing.sh")).unwrap();
    oslo::names::refresh(&env);
    assert!(!oslo_base::vocab::contains("deploy_thing"));
}

/// **Quoting a verb's name is the escape hatch when something else already owns it.**
///
/// Forty names carry structure, and a name oslo invented can still be one somebody already uses: an
/// `alias lines=tokei` shadows the verb, because aliases expand before the planner ever sees the
/// pipeline. The POSIX way out is to quote any character of the word, which suppresses the alias —
/// but `simple_command_name` accepted a *bare* literal only, so the escape that recovered the name
/// from the alias then hid it from the planner, and `\lines` answered `lines: command not found`.
///
/// The one way out of a vocabulary collision has to work.
#[test]
fn a_quoted_verb_name_still_reaches_the_planner() {
    let dir = tempfile::tempdir().expect("tempdir");

    for spelling in ["\\lines", "'lines'", "\"lines\""] {
        let run = common::run_in(dir.path(), &format!("seq 1 3 | {spelling} | length"));
        assert_eq!(
            run.out(),
            "3",
            "`{spelling}` did not reach the planner: {}",
            run.stderr
        );
    }
}

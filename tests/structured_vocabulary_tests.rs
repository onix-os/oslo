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

/// **A verb reported missing must not be reported as a `$PATH` mistake.**
///
/// `where: command not found; did you mean hexe?` was wrong in every clause: `where` exists, `$PATH`
/// is not where it lives, and `hexe` is unrelated. A verb reaches that path only when no edge of its
/// pipeline carried rows, which is a different problem with a different fix.
#[test]
fn a_verb_with_no_rows_says_what_it_is() {
    let dir = tempfile::tempdir().expect("tempdir");
    let run = common::run_in(dir.path(), "where 'true'");

    assert!(
        run.stderr.contains("a structured verb, not a command"),
        "stderr: {}",
        run.stderr
    );
    assert!(
        !run.stderr.contains("did you mean"),
        "it must not guess at $PATH: {}",
        run.stderr
    );
    assert_eq!(run.status, 127, "still the status a missing command has");
}

/// **An alias no longer shadows a verb inside a pipeline** — the reading is decided by position, so
/// the line that started all this simply works.
#[test]
fn an_alias_does_not_shadow_a_verb_in_a_pipeline() {
    let dir = tempfile::tempdir().expect("tempdir");
    let run = common::run_in(dir.path(), "alias lines=tokei\nseq 1 3 | lines | length");

    assert_eq!(run.out(), "3", "stderr: {}", run.stderr);
    assert_eq!(run.status, 0);
}

/// **And where one still can, it is named.** A verb reported missing lists the aliases that carry
/// verb names, because the word that failed is never the word that was aliased away — that one is
/// gone by the time anything can look at it.
#[test]
fn a_shadowing_alias_is_named_when_a_verb_is_reported_missing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let run = common::run_in(dir.path(), "alias length=wc\nwhere 'true'");

    assert!(
        run.stderr.contains("a structured verb, not a command"),
        "stderr: {}",
        run.stderr
    );
    assert!(
        run.stderr.contains("length") && run.stderr.contains("shadow"),
        "the shadowing alias is named and explained: {}",
        run.stderr
    );
}

/// A word that is genuinely not a command is untouched — this must not swallow the ordinary case.
#[test]
fn an_unknown_command_still_reads_as_one() {
    let dir = tempfile::tempdir().expect("tempdir");
    let run = common::run_in(dir.path(), "nosuchcommandanywhere");

    assert!(
        run.stderr.contains("command not found"),
        "stderr: {}",
        run.stderr
    );
    assert!(
        !run.stderr.contains("structured verb"),
        "it is not a verb: {}",
        run.stderr
    );
    assert_eq!(run.status, 127);
}

/// **A bridge at the end of a pipeline is a pipeline.** `Sink::Rows` describes an edge, and a
/// bridge with nothing after it has none — which used to drop `cat data.json | from json` onto the
/// byte path and report a name that is not missing. Unlike `ls`, `ps` and `df`, none of the four
/// bridges shadows a real command, so there is nothing for the fallback to reach.
#[test]
fn a_bridge_at_the_end_of_a_pipeline_still_runs() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("d.json"), r#"[{"a":1},{"a":2}]"#).expect("write");

    for (script, want) in [
        ("cat d.json | from json", "1\n2"),
        ("seq 1 3 | lines", "1\n2\n3"),
        ("printf 'x:1\\n' | parse '{k}:{v}'", "x\t1"),
    ] {
        let run = common::run_in(dir.path(), script);
        assert_eq!(run.out(), want, "{script}: stderr: {}", run.stderr);
        assert_eq!(run.status, 0, "{script}");
    }
}

/// A bare `ls`, `ps` or `df` must still be the command of that name: those four producers each
/// shadow one, and that fallback is what the bridge rule must not take with it.
#[test]
fn a_bare_producer_is_still_the_command_it_shadows() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("only"), "x").expect("write");
    let run = common::run_in(dir.path(), "ls");

    assert_eq!(run.out(), "only", "stderr: {}", run.stderr);
}

/// The last stage is the one whose redirection the structured runner applies, so a redirected
/// bridge writes the file rather than printing to the terminal and leaving it empty.
#[test]
fn a_redirected_bridge_writes_the_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let run = common::run_in(dir.path(), "seq 1 2 | lines > rows.txt");

    assert_eq!(run.out(), "", "nothing on stdout: {:?}", run.stdout);
    assert_eq!(run.status, 0, "stderr: {}", run.stderr);
    let written = std::fs::read_to_string(dir.path().join("rows.txt")).expect("the file");
    assert_eq!(written.trim_end(), "1\n2");
}

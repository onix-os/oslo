use super::*;

/// A rebuild is a difference, and this is the arithmetic that finds it.
#[test]
fn what_was_ours_and_is_no_longer_is_what_comes_off() {
    let had = Applied {
        aliases: vec!["gs".into(), "gco".into()],
        abbrevs: vec!["gc".into()],
        vars: vec!["GITHUB_TOKEN".into()],
    };
    let now = Applied {
        aliases: vec!["gs".into()],
        abbrevs: vec!["gc".into(), "gp".into()],
        vars: Vec::new(),
    };
    let gone = had.gone(&now);
    assert_eq!(gone.aliases, ["gco"], "removed from the database");
    assert!(gone.abbrevs.is_empty(), "a new one is not a removal");
    assert_eq!(gone.vars, ["GITHUB_TOKEN"], "a variable comes off too");
}

/// **An alias you typed is not ours to take away.** It was never in `Applied`, so it can never be
/// in the difference — which is the whole reason the set is remembered rather than recomputed.
#[test]
fn a_name_we_never_put_there_is_never_removed() {
    let ours = Applied {
        aliases: vec!["gs".into()],
        abbrevs: Vec::new(),
        vars: Vec::new(),
    };
    let gone = ours.gone(&Applied::default()).aliases;
    assert_eq!(gone, ["gs"]);
    assert!(
        !gone.contains(&"typed_at_the_prompt".to_string()),
        "only what we installed"
    );
}

#[test]
fn applied_reads_the_two_kinds_a_shell_holds() {
    let entries = [
        Entry::new(Kind::Alias, "gs", "git status"),
        Entry::new(Kind::Abbrev, "gco", "git checkout"),
        Entry::new(Kind::Script, "deploy", "#!/bin/sh\n"),
    ];
    let applied = Applied::of(&entries);
    assert_eq!(applied.aliases, ["gs"]);
    assert_eq!(applied.abbrevs, ["gco"]);
    assert_eq!(
        applied.aliases.len() + applied.abbrevs.len(),
        2,
        "a script is not something a starting shell holds"
    );
}

/// Two `stat`s and no read. The comparison is the whole point: an unchanged pair does nothing.
#[test]
fn stamps_compare_rather_than_read() {
    let first = Stamps::now();
    assert_eq!(first, Stamps::now(), "nothing moved between these lines");
}

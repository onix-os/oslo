//! An alias completes as the command it expands to — all of it.
//!
//! `alias gco='git checkout'` used to complete as plain `git`: only the *first word* of the
//! expansion reached the spec lookup, so `gco -<Tab>` offered git's top-level options —
//! `--version`, `-C` — where `git checkout -<Tab>` offers `--force` and `-b`. The subcommand the
//! alias exists to name was dropped, and everybody aliases `gco`.

use oslo::env::Environment;
use oslo::ui::OsloHelper;
use std::sync::{Arc, Mutex};

fn helper_with(aliases: &[(&str, &str)]) -> OsloHelper {
    let mut env = Environment::new();
    for (name, body) in aliases {
        env.set_alias(name, body);
    }
    let mut h = OsloHelper::new(Arc::new(Mutex::new(env)));
    h.set_menu(false);
    h
}

fn options(h: &OsloHelper, line: &str) -> Vec<String> {
    let (_, candidates) = h.candidates(line, line.len());
    let mut names: Vec<String> = candidates.into_iter().map(|c| c.display).collect();
    names.sort();
    names
}

/// **The case this exists for.** The alias and what it stands for offer the same options.
#[test]
fn an_alias_completes_as_the_whole_command_it_names() {
    let h = helper_with(&[("gco", "git checkout")]);

    let direct = options(&h, "git checkout -");
    let aliased = options(&h, "gco -");
    assert!(
        !direct.is_empty(),
        "the git spec should offer checkout's options"
    );
    assert_eq!(aliased, direct, "an alias is the command it expands to");
    assert!(
        direct.iter().any(|o| o == "--force"),
        "checkout's own options, not git's: {direct:?}"
    );
    assert!(
        !aliased.iter().any(|o| o == "--version"),
        "`--version` is git's top-level option, not checkout's: {aliased:?}"
    );
}

/// **Aliases chain, and the words accumulate in order.** `alias g=git` with `alias gc='g commit'`
/// has to reach `git commit`, not `git` alone and not `commit git`.
#[test]
fn a_chained_alias_keeps_the_words_in_order() {
    let h = helper_with(&[("g", "git"), ("gc", "g commit")]);

    let direct = options(&h, "git commit -");
    let aliased = options(&h, "gc -");
    assert!(
        !direct.is_empty(),
        "the git spec should offer commit's options"
    );
    assert_eq!(aliased, direct);
}

/// A self-referencing alias is the classic one, and it must not loop.
#[test]
fn a_self_referencing_alias_terminates() {
    let h = helper_with(&[("ls", "ls --color")]);
    // Answering at all is the assertion: before the loop guard this would not return.
    let _ = options(&h, "ls -");

    let h = helper_with(&[("a", "b"), ("b", "a")]);
    let _ = options(&h, "a -");
}

/// An ordinary command is untouched by any of this.
#[test]
fn a_command_that_is_not_an_alias_is_unchanged() {
    let plain = helper_with(&[]);
    let with_alias = helper_with(&[("gco", "git checkout")]);
    assert_eq!(
        options(&plain, "git checkout -"),
        options(&with_alias, "git checkout -")
    );
}

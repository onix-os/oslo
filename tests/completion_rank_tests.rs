//! Fuzzy matching that reaches a candidate must also *rank* it.
//!
//! Every candidate in a fuzzy result set matched, so frecency cannot tell them apart — and unrun
//! commands all score zero, which left the dropdown in plain alphabetical order. `gco` found
//! `git checkout` and then buried it, which is the whole selling point of the `smart` preset.

use oslo::env::Environment;
use oslo::ui::OsloHelper;
use oslo::ui::matching::Fuzzy;
use oslo::ui::settings::{self, Settings};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::{Arc, Mutex};

fn make_exe(dir: &Path, name: &str) {
    let p = dir.join(name);
    fs::write(&p, b"#!/bin/sh\n").unwrap();
    fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
}

/// A fuzzy match that scores higher is offered first, whatever the alphabet says.
#[test]
fn a_better_fuzzy_match_is_offered_first() {
    let dir = tempfile::tempdir().unwrap();
    // Alphabetically `qaxbxc` comes first; by fuzzy score `qzabc` — the tighter run — wins.
    make_exe(dir.path(), "qaxbxc");
    make_exe(dir.path(), "qzabc");

    let mut s = Settings::default();
    s.completion.fuzzy = Fuzzy::Smart;
    settings::install(s);

    let mut env = Environment::new();
    env.set_var("PATH", dir.path().to_str().unwrap(), false);
    let mut h = OsloHelper::new(Arc::new(Mutex::new(env)));
    h.set_menu(false);

    let (_, cands) = h.candidates("qabc", 4);
    let names: Vec<&str> = cands.iter().map(|c| c.display.as_str()).collect();
    assert!(names.len() >= 2, "both should match: {names:?}");
    assert_eq!(
        names[0], "qzabc",
        "the better fuzzy match must come first, not the alphabetical one: {names:?}"
    );

    settings::install(Settings::default());
}

/// **`${NAME` is a parameter expansion, not a brace list.** Counted as a list, the word was
/// retargeted past the `{` with the `$` folded into its stem, and taking the candidate spliced
/// `echo ${$HOME` into the line — text nobody typed, on the ordinary way of writing `${HOME}`.
#[test]
fn a_braced_variable_completes_and_closes_its_brace() {
    let mut env = Environment::new();
    env.set_var("OSLO_BRACE_PROBE", "x", false);
    let mut h = OsloHelper::new(Arc::new(Mutex::new(env)));
    h.set_menu(false);

    let line = "echo ${OSLO_BRACE_PRO";
    let (start, cands) = h.candidates(line, line.len());
    let shown: Vec<&str> = cands.iter().map(|c| c.replacement.as_str()).collect();
    assert_eq!(
        shown,
        vec!["${OSLO_BRACE_PROBE}"],
        "the brace has to be closed too: {shown:?}"
    );
    assert_eq!(
        start,
        line.find("${").expect("the sigil"),
        "the replacement starts at the `$`, not inside the brace"
    );
}

/// **A command written as a path completes as one.** Nothing on `$PATH` is reached by `./bui`, so
/// the name builders can never answer and Tab did nothing at all — while the highlighter coloured
/// the same word as a real command the moment it was finished.
#[test]
fn a_command_with_a_slash_completes_as_a_path() {
    let dir = tempfile::tempdir().unwrap();
    make_exe(dir.path(), "build.sh");

    let h = {
        let env = Environment::new();
        let mut h = OsloHelper::new(Arc::new(Mutex::new(env)));
        h.set_menu(false);
        h
    };

    let line = format!("{}/bui", dir.path().display());
    let (_, cands) = h.candidates(&line, line.len());
    let shown: Vec<&str> = cands.iter().map(|c| c.display.as_str()).collect();
    assert!(shown.contains(&"build.sh"), "{shown:?}");
}

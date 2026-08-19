//! The tilde forms the shell expands, and whether the prompt knows them.
//!
//! oslo expands `~`, `~user`, `~+` and `~-` exactly as bash does. Completion, the ghost suggestion
//! and the highlighter each knew only `~` and `~/…` — so `ls ~+/` offered nothing, `~root` read as
//! a path that is not there, and three layers disagreed with the shell about a form it handles
//! perfectly. They now share the shell's own expander.

use oslo::env::Environment;
use oslo::ui::OsloHelper;
use oslo::ui::highlight::{Context, TokenType, classify, lex};
use std::sync::{Arc, Mutex};

fn helper() -> OsloHelper {
    let mut h = OsloHelper::new(Arc::new(Mutex::new(Environment::new())));
    h.set_menu(false);
    h
}

/// **`~+` is the working directory**, and Tab had nothing to say about it.
#[test]
fn the_working_directory_tilde_completes() {
    let h = helper();
    let line = "ls ~+/Car";
    let (_, candidates) = h.candidates(line, line.len());
    let shown: Vec<&str> = candidates.iter().map(|c| c.display.as_str()).collect();
    assert!(
        shown.contains(&"Cargo.toml"),
        "`~+/` is this directory: {shown:?}"
    );

    // **And it is written back as `~+`.** Replacing the tilde with what it stood for would rewrite
    // a line the user deliberately made portable.
    let written: Vec<&str> = candidates.iter().map(|c| c.replacement.as_str()).collect();
    assert!(
        written.iter().all(|w| w.starts_with("~+/")),
        "the tilde survives: {written:?}"
    );
}

/// `~user` reaches that user's home, for a user whose home can actually be read.
#[test]
fn a_named_user_tilde_completes() {
    let Ok(user) = std::env::var("USER") else {
        return;
    };
    if user.is_empty() {
        return;
    }
    let h = helper();
    let line = format!("ls ~{user}/");
    let (_, candidates) = h.candidates(&line, line.len());
    assert!(
        !candidates.is_empty(),
        "`~{user}/` is a home directory with things in it"
    );
    assert!(
        candidates
            .iter()
            .all(|c| c.replacement.starts_with(&format!("~{user}/"))),
        "written back as typed"
    );
}

/// The ghost reaches them too — it was the same `~`-only rule in a second place.
#[test]
fn the_ghost_knows_the_working_directory_tilde() {
    let h = helper();
    assert!(
        h.path_hint("ls ~+/Car", 9).is_some(),
        "`~+/Car` continues to a real file"
    );
}

/// **A tilde form the shell resolves is a path that is there.** `~root` read as a mistake.
#[test]
fn a_tilde_path_is_lit_as_one_that_exists() {
    let no = |_: &str| false;
    let ctx = Context {
        path: "",
        is_builtin: &no,
        is_function: &no,
        check_paths: true,
    };
    let valid = |line: &str| {
        classify(&lex(line), &ctx)
            .iter()
            .any(|(_, t)| *t == TokenType::ValidPath)
    };

    assert!(valid("ls ~root"), "root has a home directory");
    assert!(valid("ls ~/"), "and so do you");
    // A name nobody can resolve stays literal, so it is not a path that exists.
    assert!(!valid("ls ~nosuchuser-xyzzy"));
}

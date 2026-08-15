//! Globs and brace lists at the prompt.
//!
//! `rm /one/tw*` is a line whose whole risk is *which files it hits*, and until this existed Tab
//! answered nothing at all: the word was looked up as a literal prefix, and no file is named `tw*`.
//! The same word reached the highlighter in pieces — `tw` then `*` — so the colour was decided by
//! asking whether a file called `tw` exists, which is a question nobody asked.

use oslo::env::Environment;
use oslo::ui::OsloHelper;
use oslo::ui::highlight::{Context, TokenType, classify, lex};
use std::sync::{Arc, Mutex};

/// A tree with two `two-*` files, an `other`, and a `three/` beside it.
fn tree() -> tempfile::TempDir {
    let root = tempfile::tempdir().expect("tempdir");
    for dir in ["one", "three"] {
        std::fs::create_dir_all(root.path().join(dir)).expect("mkdir");
    }
    for file in [
        "one/two-alpha",
        "one/two-beta",
        "one/other",
        "three/four",
        "three/five",
    ] {
        std::fs::write(root.path().join(file), b"x").expect("write");
    }
    root
}

fn helper() -> OsloHelper {
    let mut h = OsloHelper::new(Arc::new(Mutex::new(Environment::new())));
    h.set_menu(false);
    h
}

fn offers(line: &str) -> Vec<String> {
    let (_, candidates) = helper().candidates(line, line.len());
    let mut names: Vec<String> = candidates.into_iter().map(|c| c.display).collect();
    names.sort();
    names
}

/// **The case this exists for.** Tab on a glob shows what the glob matches.
#[test]
fn a_glob_offers_what_it_matches() {
    let root = tree();
    let base = root.path().display();

    let line = format!("rm {base}/one/tw*");
    assert_eq!(offers(&line), vec!["two-alpha", "two-beta"], "{line}");

    // And the replacement is the whole path, so accepting one leaves a line that runs.
    let (start, candidates) = helper().candidates(&line, line.len());
    let first = candidates.first().expect("a candidate");
    assert_eq!(start, line.find(&format!("{base}")).expect("the word"));
    assert!(
        first.replacement.ends_with("two-alpha"),
        "{:?}",
        first.replacement
    );
    assert!(
        first.replacement.starts_with(&format!("{base}/one/")),
        "{:?}",
        first.replacement
    );
}

/// **A `*` in the directory part too.** `read_dir("/x/*/")` simply fails, so this offered nothing.
#[test]
fn a_glob_in_the_directory_part_is_walked() {
    let root = tree();
    let base = root.path().display();

    let line = format!("ls {base}/*/fo");
    assert_eq!(offers(&line), vec!["four"], "{line}");
}

/// **A quoted glob is a filename, not a pattern.** The shell will not expand it, so completion must
/// not offer what it would have matched — the two would describe different commands.
#[test]
fn a_quoted_glob_is_not_a_pattern() {
    let root = tree();
    let base = root.path().display();

    let line = format!("rm \"{base}/one/tw*\"");
    assert!(
        offers(&line).is_empty(),
        "a quoted star names a file called `tw*`, which is not there"
    );
}

/// Adding an item to a brace list completes against the same directory the list is in.
#[test]
fn a_brace_list_completes_its_next_item() {
    let root = tree();
    let base = root.path().display();

    let line = format!("rm {base}/three/{{four,fi");
    assert_eq!(offers(&line), vec!["five"], "{line}");

    // And a fresh list offers everything in that directory.
    let line = format!("rm {base}/three/{{four,");
    assert_eq!(offers(&line), vec!["five", "four"], "{line}");
}

/// **The highlighter resolves a globbed word whole.** In pieces, `tw*` asked whether a file called
/// `tw` exists — nobody typed that — so a glob that matches half the directory read the same as one
/// that matches nothing.
#[test]
fn a_glob_is_coloured_by_what_it_matches() {
    let root = tree();
    let base = root.path().display();
    let no = |_: &str| false;
    let ctx = Context {
        path: "",
        is_builtin: &no,
        is_function: &no,
        check_paths: true,
    };

    let hits = format!("rm {base}/one/tw*");
    let kinds = classify(&lex(&hits), &ctx);
    assert!(
        kinds.iter().any(|(_, t)| *t == TokenType::ValidPath),
        "a glob that matches should read as a path that is there: {kinds:?}"
    );

    let misses = format!("rm {base}/one/zz*");
    let kinds = classify(&lex(&misses), &ctx);
    assert!(
        !kinds.iter().any(|(_, t)| *t == TokenType::ValidPath),
        "a glob that matches nothing must not: {kinds:?}"
    );
}

/// The star itself keeps its own colour either way — whether a word will expand is the thing you
/// most want to see before pressing Enter.
#[test]
fn the_star_is_still_lit_as_a_glob() {
    let root = tree();
    let base = root.path().display();
    let no = |_: &str| false;
    let ctx = Context {
        path: "",
        is_builtin: &no,
        is_function: &no,
        check_paths: true,
    };

    let line = format!("rm {base}/one/tw*");
    let kinds = classify(&lex(&line), &ctx);
    assert!(
        kinds
            .iter()
            .any(|(text, t)| text == "*" && *t == TokenType::Glob),
        "{kinds:?}"
    );
}

/// The directory walk is not fooled by a path that is not there.
#[test]
fn a_glob_that_reaches_nothing_offers_nothing() {
    let root = tree();
    let base = root.path().display();
    assert!(offers(&format!("ls {base}/nosuch/*")).is_empty());
    assert!(offers(&format!("ls {base}/one/zz*")).is_empty());
}

/// **A quoted `@name` is a literal, and completion must not pretend otherwise.**
///
/// The expander only substitutes an unquoted `@name`, so offering a mark inside quotes promises an
/// expansion that will never happen. The symptom was worse than a wrong candidate: the mark builder
/// writes back as though the word were bare, so `ls "@pr` + Tab returned `ls @proj/` — the opening
/// quote deleted, and with a closing quote present the line fell to a continuation prompt.
#[test]
fn a_quoted_mark_is_not_completed_as_a_mark() {
    let root = tree();
    oslo_base::dirs::set_named_dirs(
        [(
            "marked".to_string(),
            root.path().join("three").display().to_string(),
        )]
        .into_iter()
        .collect(),
    );

    assert!(
        offers("ls @mark")
            .iter()
            .any(|name| name.contains("marked")),
        "an unquoted mark still completes: {:?}",
        offers("ls @mark")
    );
    assert!(
        !offers("ls \"@mark")
            .iter()
            .any(|name| name.contains("marked")),
        "a quoted mark must not be offered: {:?}",
        offers("ls \"@mark")
    );

    oslo_base::dirs::set_named_dirs(std::collections::HashMap::new());
}

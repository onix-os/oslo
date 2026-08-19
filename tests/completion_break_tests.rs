//! Where a word ends, for completion.
//!
//! A word as the lexer sees it is not always the thing Tab should complete. `--file=arch` is one
//! word and two questions; `PATH=/usr/bin:/us` is one word and three. bash breaks such words on
//! the characters in `COMP_WORDBREAKS` and completes the last piece, and until this existed oslo
//! did not — every word containing an `=` or a `:` was looked up whole, matched nothing, and Tab
//! did nothing at all. It is the bug this file was written for.

use oslo::env::Environment;
use oslo::ui::OsloHelper;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::{Arc, Mutex};

/// A helper over `env` with the dropdown off — see the note in `interactive_tests.rs`.
fn helper(env: Environment) -> OsloHelper {
    let mut h = OsloHelper::new(Arc::new(Mutex::new(env)));
    h.set_menu(false);
    h
}

/// An environment whose `$PATH` is exactly `dir`, set **unexported**: exporting would rewrite this
/// test process's real `environ` from a libtest worker thread. See `interactive_tests.rs`.
fn env_with_path(dir: &Path) -> Environment {
    let mut env = Environment::new();
    env.set_var("PATH", dir.to_str().unwrap(), false);
    env
}

fn make_exe(dir: &Path, name: &str) {
    let p = dir.join(name);
    fs::write(&p, b"#!/bin/sh\n").unwrap();
    fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
}

fn displays(h: &OsloHelper, line: &str, pos: usize) -> Vec<String> {
    let (_, cands) = h.candidates(line, pos);
    cands.into_iter().map(|c| c.display).collect()
}

/// **A word with an `=` in it completes after the `=`.**
///
/// `=` is a word break for completion, as it is in bash. Before this, every word containing one
/// completed against nothing at all: the whole of `--file=some` was looked up as a path, no such
/// file exists, and Tab did nothing — for `dd if=`, for `--prefix=/usr/lo`, for any long option
/// that takes a value.
#[test]
fn a_word_completes_after_its_equals_sign() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("archive.tar"), b"x").unwrap();
    let h = helper(Environment::new());
    let base = dir.path().display();

    let line = format!("tar --file={base}/arch");
    let (start, cands) = h.candidates(&line, line.len());
    let names: Vec<&str> = cands.iter().map(|c| c.display.as_str()).collect();
    assert_eq!(names, vec!["archive.tar"], "{names:?}");
    assert_eq!(
        start,
        line.find("--file=").unwrap() + "--file=".len(),
        "the replacement has to land after the `=`, not over it"
    );
}

/// The bare `=name` the user hit this with: a word that is nothing but `=` and a stem.
#[test]
fn a_word_that_begins_with_equals_completes_too() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("something"), b"x").unwrap();
    let h = helper(Environment::new());
    let line = format!("ls ={}/someth", dir.path().display());
    let (_, cands) = h.candidates(&line, line.len());
    let names: Vec<&str> = cands.iter().map(|c| c.display.as_str()).collect();
    assert_eq!(names, vec!["something"], "{names:?}");
}

/// The **last** `=`, so a value that itself contains one is left alone.
#[test]
fn only_the_last_equals_breaks_the_word() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("target"), b"x").unwrap();
    let h = helper(Environment::new());
    let line = format!("cmd --set=a=b={}/targ", dir.path().display());
    let (start, cands) = h.candidates(&line, line.len());
    let names: Vec<&str> = cands.iter().map(|c| c.display.as_str()).collect();
    assert_eq!(names, vec!["target"], "{names:?}");
    assert_eq!(start, line.rfind('=').unwrap() + 1);
}

/// **A quoted `=` is a character in a filename, not a word break.**
///
/// Quoting is how you say "this is one word", and a shell that split there anyway would take the
/// tool away from the person who reached for it.
#[test]
fn a_quoted_equals_does_not_break_the_word() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("a=b.txt"), b"x").unwrap();
    let h = helper(Environment::new());
    let line = format!("cat \"{}/a=b", dir.path().display());
    let (_, cands) = h.candidates(&line, line.len());
    let names: Vec<&str> = cands.iter().map(|c| c.display.as_str()).collect();
    assert_eq!(names, vec!["a=b.txt"], "{names:?}");
}

/// An assignment still leads a command, and the command after it still completes.
#[test]
fn an_assignment_prefix_still_completes_the_command_after_it() {
    let dir = tempfile::tempdir().unwrap();
    make_exe(dir.path(), "lsx");
    let h = helper(env_with_path(dir.path()));
    let names = displays(&h, "FOO=1 ls", 8);
    assert!(names.iter().any(|n| n == "lsx"), "{names:?}");
}

/// And the *value* of a leading assignment is completed as a path, not as a command name.
///
/// `FOO=val` is an assignment; the text after the `=` is a value. Offering the commands on `$PATH`
/// that start with `val` would be answering a question nobody asked — and it is what the word
/// carried before, because a leading assignment is still the line's first word.
#[test]
fn the_value_of_an_assignment_completes_as_a_path_not_a_command() {
    let bin = tempfile::tempdir().unwrap();
    make_exe(bin.path(), "valuecommand");
    let files = tempfile::tempdir().unwrap();
    fs::write(files.path().join("valuefile"), b"x").unwrap();

    let h = helper(env_with_path(bin.path()));
    let line = format!("FOO={}/value", files.path().display());
    let (_, cands) = h.candidates(&line, line.len());
    let names: Vec<&str> = cands.iter().map(|c| c.display.as_str()).collect();
    assert_eq!(names, vec!["valuefile"], "{names:?}");
}

/// **`:` breaks a word too**, which is what makes editing a `$PATH` bearable.
///
/// `FOO=/usr/bin:/us` is the everyday case; `scp host:/pa` is the other one. Both completed
/// against nothing before, because the whole word was looked up as a single path.
#[test]
fn a_colon_breaks_a_word_as_well() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("colondir")).unwrap();
    let h = helper(Environment::new());

    let line = format!("PATH=/usr/bin:{}/colond", dir.path().display());
    let (start, cands) = h.candidates(&line, line.len());
    let names: Vec<&str> = cands.iter().map(|c| c.display.as_str()).collect();
    assert_eq!(names, vec!["colondir/"], "{names:?}");
    assert_eq!(start, line.rfind(':').unwrap() + 1, "the `:` is kept");
}

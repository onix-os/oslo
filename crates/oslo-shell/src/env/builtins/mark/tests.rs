//! What `mark` does to the mark file, and to itself when typed twice.

use super::*;
use std::sync::{Mutex, MutexGuard};

/// The marks file is one path for the whole process, so these take turns.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

fn alone() -> (MutexGuard<'static, ()>, tempfile::TempDir) {
    let guard = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().expect("tempdir");
    dirs::set_marks_file(Some(dir.path().join("marks")));
    dirs::set_named_dirs(std::collections::HashMap::new());
    (guard, dir)
}

/// A shell standing in `where`, without moving the test process.
fn standing_in(where_: &str) -> Environment {
    let mut env = Environment::new();
    env.set_var("PWD", where_, false);
    env
}

/// Called the way the dispatcher calls it: `args[0]` is the name it was reached by.
fn run(env: &mut Environment, args: &[&str]) -> i32 {
    let mut argv = vec!["mark".to_string()];
    argv.extend(args.iter().map(|a| a.to_string()));
    builtin_mark(env, &argv).expect("the builtin never fails outright")
}

/// **The case it exists for.** `mark` marks, `mark` again unmarks, and the name is the directory's own.
#[test]
fn typing_it_twice_undoes_it() {
    let (_lock, _tmp) = alone();
    let mut env = standing_in("/home/u/data/code/rush");

    assert_eq!(run(&mut env, &[]), 0);
    assert_eq!(
        dirs::named_dir("rush").as_deref(),
        Some("/home/u/data/code/rush")
    );

    assert_eq!(run(&mut env, &[]), 0);
    assert_eq!(
        dirs::named_dir("rush"),
        None,
        "the second `mark` took it back"
    );

    dirs::set_marks_file(None);
}

/// A name of your own, and typing `mark` again still undoes it — the *path* is what the toggle is
/// about, so a mark made under any name is the one a bare `mark` here removes.
#[test]
fn a_chosen_name_is_still_undone_by_a_bare_mark() {
    let (_lock, _tmp) = alone();
    let mut env = standing_in("/home/u/work/app");

    assert_eq!(run(&mut env, &["proj"]), 0);
    assert_eq!(dirs::named_dir("proj").as_deref(), Some("/home/u/work/app"));
    assert_eq!(
        dirs::named_dir("app"),
        None,
        "the chosen name, not the basename"
    );

    assert_eq!(run(&mut env, &[]), 0);
    assert_eq!(dirs::named_dir("proj"), None);

    dirs::set_marks_file(None);
}

/// Naming a directory that is already marked renames it rather than leaving two names for one path,
/// which is a state the listing cannot explain and a bare `mark` could not undo.
#[test]
fn naming_an_already_marked_directory_renames_it() {
    let (_lock, _tmp) = alone();
    let mut env = standing_in("/home/u/work/app");

    assert_eq!(run(&mut env, &[]), 0);
    assert_eq!(dirs::named_dir("app").as_deref(), Some("/home/u/work/app"));

    assert_eq!(run(&mut env, &["proj"]), 0);
    assert_eq!(dirs::named_dir("app"), None, "the old name is gone");
    assert_eq!(dirs::named_dir("proj").as_deref(), Some("/home/u/work/app"));
    assert_eq!(dirs::marks().len(), 1, "one row, not two");

    dirs::set_marks_file(None);
}

/// **A name that means somewhere else is refused.** The bare `mark` chose this name rather than being
/// told it, so walking into a second `src` must not silently move the first one.
#[test]
fn a_name_already_taken_is_refused() {
    let (_lock, _tmp) = alone();

    let mut first = standing_in("/home/u/one/src");
    assert_eq!(run(&mut first, &[]), 0);

    let mut second = standing_in("/home/u/two/src");
    assert_eq!(run(&mut second, &[]), 1, "refused rather than overwritten");
    assert_eq!(dirs::named_dir("src").as_deref(), Some("/home/u/one/src"));

    // And saying which name to use is how you get past it.
    assert_eq!(run(&mut second, &["src2"]), 0);
    assert_eq!(dirs::named_dir("src2").as_deref(), Some("/home/u/two/src"));

    dirs::set_marks_file(None);
}

/// `-d` reaches a mark from anywhere, which the toggle cannot: it is about the directory you are in.
#[test]
fn delete_reaches_a_mark_from_elsewhere() {
    let (_lock, _tmp) = alone();
    let mut env = standing_in("/home/u/work/app");
    assert_eq!(run(&mut env, &[]), 0);

    let mut elsewhere = standing_in("/tmp");
    assert_eq!(run(&mut elsewhere, &["-d", "app"]), 0);
    assert_eq!(dirs::named_dir("app"), None);
    // Saying so when there was nothing to forget, rather than reporting success.
    assert_eq!(run(&mut elsewhere, &["-d", "app"]), 1);
    assert_eq!(run(&mut elsewhere, &["-d"]), 2, "and -d needs a name");

    dirs::set_marks_file(None);
}

/// An unknown option is refused rather than taken for a name, so a typo cannot become a mark.
#[test]
fn an_unknown_option_is_not_a_name() {
    let (_lock, _tmp) = alone();
    let mut env = standing_in("/home/u/work/app");

    assert_eq!(run(&mut env, &["--nope"]), 2);
    assert!(dirs::marks().is_empty());

    dirs::set_marks_file(None);
}

/// The root has no last component to be called by, so it says so instead of marking a nameless one.
#[test]
fn a_directory_with_no_name_asks_for_one() {
    let (_lock, _tmp) = alone();
    let mut env = standing_in("/");

    assert_eq!(run(&mut env, &[]), 1);
    assert!(dirs::marks().is_empty());
    // Told a name, it is an ordinary mark.
    assert_eq!(run(&mut env, &["root"]), 0);
    assert_eq!(dirs::named_dir("root").as_deref(), Some("/"));

    dirs::set_marks_file(None);
}

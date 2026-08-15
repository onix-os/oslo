//! The two layers of `@name`, and the file the marked one lives in.

use super::*;
use std::sync::{Mutex, MutexGuard};

/// The marks file is one path for the whole process, so these take turns.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

/// A private marks file, and the lock that keeps two tests out of each other's.
fn alone() -> (MutexGuard<'static, ()>, tempfile::TempDir) {
    let guard = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().expect("tempdir");
    set_marks_file(Some(dir.path().join("marks")));
    set_named_dirs(HashMap::new());
    (guard, dir)
}

#[test]
fn a_declared_name_expands_and_keeps_its_tail() {
    let (_lock, _dir) = alone();
    set_named_dirs(HashMap::from([(
        "work".to_string(),
        "/home/u/work".to_string(),
    )]));

    assert_eq!(expand_at("work").as_deref(), Some("/home/u/work"));
    assert_eq!(
        expand_at("work/src/main.rs").as_deref(),
        Some("/home/u/work/src/main.rs")
    );
    assert_eq!(expand_at("nowhere"), None);
    assert_eq!(expand_at(""), None);
    assert_eq!(named_dirs(), vec![("work".into(), "/home/u/work".into())]);

    set_named_dirs(HashMap::new());
    set_marks_file(None);
}

/// **A mark outlives the shell that made it**, which is the whole point: it is written to a file
/// and read back by whatever asks next, including another terminal.
#[test]
fn a_mark_is_written_and_read_back() {
    let (_lock, _dir) = alone();

    assert_eq!(marks(), Vec::new(), "nothing marked yet");
    mark("proj", "/tmp/proj").expect("mark");

    assert_eq!(named_dir("proj").as_deref(), Some("/tmp/proj"));
    assert_eq!(expand_at("proj/src").as_deref(), Some("/tmp/proj/src"));
    assert_eq!(mark_of("/tmp/proj").as_deref(), Some("proj"));
    assert_eq!(marks(), vec![("proj".into(), "/tmp/proj".into())]);

    assert!(unmark("proj").expect("unmark"), "it was there");
    assert!(!unmark("proj").expect("unmark"), "and now it is not");
    assert_eq!(named_dir("proj"), None);
    assert_eq!(mark_of("/tmp/proj"), None);

    set_marks_file(None);
}

/// **A declared name wins.** `oslo.dirs` is a statement a config made; a mark is something typed in
/// passing, and it must not quietly take over a name the config decided.
#[test]
fn a_declared_name_beats_a_mark_of_the_same_name() {
    let (_lock, _dir) = alone();
    mark("work", "/tmp/marked").expect("mark");
    set_named_dirs(HashMap::from([(
        "work".to_string(),
        "/tmp/declared".to_string(),
    )]));

    assert_eq!(named_dir("work").as_deref(), Some("/tmp/declared"));
    // And it appears once in the listing, not twice.
    assert_eq!(named_dirs(), vec![("work".into(), "/tmp/declared".into())]);

    set_named_dirs(HashMap::new());
    set_marks_file(None);
}

/// Both layers reach the completion listing, sorted, so `@<Tab>` offers everything there is.
#[test]
fn the_listing_is_both_layers() {
    let (_lock, _dir) = alone();
    mark("beta", "/tmp/b").expect("mark");
    mark("alpha", "/tmp/a").expect("mark");
    set_named_dirs(HashMap::from([("zeta".to_string(), "/tmp/z".to_string())]));

    assert_eq!(
        named_dirs(),
        vec![
            ("alpha".into(), "/tmp/a".into()),
            ("beta".into(), "/tmp/b".into()),
            ("zeta".into(), "/tmp/z".into()),
        ]
    );

    set_named_dirs(HashMap::new());
    set_marks_file(None);
}

/// A name has to lex as one word where it is typed, or `@name` could never find it again.
#[test]
fn a_name_that_could_not_be_typed_is_refused() {
    let (_lock, _dir) = alone();

    for bad in ["", "with space", "a/b", "-dash", "quote'd", "$var", "@at"] {
        assert!(!valid_mark_name(bad), "{bad:?} should be refused");
        assert!(mark(bad, "/tmp/x").is_err(), "{bad:?} should not store");
    }
    for good in ["proj", "my-proj", "my_proj", "v1.2", "c++"] {
        assert!(valid_mark_name(good), "{good:?} should be a name");
    }
    // A path the one-line-each format cannot hold is refused rather than silently truncated.
    assert!(mark("ok", "/tmp/two\nlines").is_err());
    assert!(mark("ok", "/tmp/a\tb").is_err());

    set_marks_file(None);
}

/// Marking a name that already exists moves it, rather than leaving two rows to disagree.
#[test]
fn marking_a_name_again_moves_it() {
    let (_lock, _dir) = alone();
    mark("proj", "/tmp/one").expect("mark");
    mark("proj", "/tmp/two").expect("mark");

    assert_eq!(named_dir("proj").as_deref(), Some("/tmp/two"));
    assert_eq!(marks().len(), 1, "one row, not two");

    set_marks_file(None);
}

/// A `~` in the file is a home-relative mark, so the file survives being copied between machines.
#[test]
fn a_tilde_in_the_file_is_resolved() {
    let (_lock, dir) = alone();
    std::fs::write(dir.path().join("marks"), "home\t~/somewhere\n").expect("write");

    let home = std::env::var("HOME").unwrap_or_default();
    if !home.is_empty() {
        assert_eq!(named_dir("home"), Some(format!("{home}/somewhere")));
    }

    set_marks_file(None);
}

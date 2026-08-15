//! Commands that refuse anything but a directory, and whether the prompt knows which they are.
//!
//! Completion had the list and the ghost did not, so `cd a` was offered `azzz/` by Tab and
//! suggested `aa` — a file — by the ghost, which `cd` then refused. It read as correct only while
//! the directory happened to sort before the file. `nav` was in neither list, though it answers
//! `not a directory` for a file exactly as `cd` does.

use oslo::env::Environment;
use oslo::ui::OsloHelper;
use std::sync::{Arc, Mutex};

/// A file that sorts *before* a directory and is shorter, so "shortest, then alphabetical" picks
/// the file unless a directory-only rule exists. That is what makes this test discriminating.
fn tree() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::write(dir.path().join("aa"), b"x").expect("write");
    std::fs::create_dir_all(dir.path().join("azzz")).expect("mkdir");
    dir
}

fn helper() -> OsloHelper {
    let mut h = OsloHelper::new(Arc::new(Mutex::new(Environment::new())));
    h.set_menu(false);
    h
}

/// **Tab and the ghost agree**, and both offer only the directory.
#[test]
fn a_directory_only_command_is_offered_only_directories() {
    let root = tree();
    let base = root.path().display();
    let h = helper();

    for command in ["cd", "pushd", "rmdir", "nav"] {
        let line = format!("{command} {base}/a");
        let (_, candidates) = h.candidates(&line, line.len());
        let shown: Vec<&str> = candidates.iter().map(|c| c.display.as_str()).collect();
        assert_eq!(shown, vec!["azzz/"], "{command}: Tab offered a file");

        assert_eq!(
            h.path_hint(&line, line.len()).as_deref(),
            Some("zzz/"),
            "{command}: the ghost suggested something the command refuses"
        );
    }
}

/// Everything else still sees both, or the rule would be a different bug.
#[test]
fn an_ordinary_command_still_sees_files() {
    let root = tree();
    let base = root.path().display();
    let h = helper();

    let line = format!("ls {base}/a");
    let (_, candidates) = h.candidates(&line, line.len());
    let shown: Vec<&str> = candidates.iter().map(|c| c.display.as_str()).collect();
    assert_eq!(shown, vec!["aa", "azzz/"]);
    assert_eq!(h.path_hint(&line, line.len()).as_deref(), Some("a"));
}

use super::*;
use crate::track::Track;
use crate::track::db::fixture::ran;

/// A flat-layout store with one command in it, plus a model file beside it.
fn old_profile(root: &Path, name: &str, line: &'static str) {
    std::fs::create_dir_all(root).expect("root");
    let store = Track::open(&root.join(format!("{name}.kv"))).expect("open");
    store.record(&ran("/w", line, 0));
    std::fs::write(root.join(format!("{name}.model")), b"a model").expect("model");
}

fn data_home(dir: &Path) -> String {
    dir.to_str().expect("utf-8").to_string()
}

#[test]
fn a_flat_store_is_copied_into_its_own_directory() {
    let temp = tempfile::tempdir().expect("tempdir");
    let data = data_home(temp.path());
    let root = temp.path().join("oslo");
    old_profile(&root, "claude", "echo from the old layout");

    let brought = from_flat_layout_in(Some(&data), None);
    assert_eq!(brought.moved, vec!["claude".to_string()]);
    assert!(brought.failed.is_empty());

    let new = root.join("history/claude");
    assert!(new.join("hist.db").is_file(), "the store moved");
    assert!(new.join("hist.model").is_file(), "and so did the model");

    // What moved is a database, not bytes: it opens and still knows the line.
    let store = Track::open(&new.join("hist.db")).expect("the copy opens");
    assert!(
        store
            .commands(10)
            .iter()
            .any(|c| c.line == "echo from the old layout"),
        "the history came with it"
    );
}

/// **Copied, not moved.** A shell running the old binary still has the old file open and will keep
/// writing to it until it exits.
#[test]
fn the_old_files_are_left_where_they_are() {
    let temp = tempfile::tempdir().expect("tempdir");
    let data = data_home(temp.path());
    let root = temp.path().join("oslo");
    old_profile(&root, "default", "echo kept");

    from_flat_layout_in(Some(&data), None);
    assert!(root.join("default.kv").is_file(), "the old store stays");
    assert!(root.join("default.model").is_file());
}

/// **Once.** Running again must not overwrite a store that has been written to since.
#[test]
fn a_profile_that_has_already_moved_is_left_alone() {
    let temp = tempfile::tempdir().expect("tempdir");
    let data = data_home(temp.path());
    let root = temp.path().join("oslo");
    old_profile(&root, "default", "echo old");

    assert_eq!(from_flat_layout_in(Some(&data), None).moved, ["default"]);

    // Something the new store learnt after the copy.
    let new = root.join("history/default/hist.db");
    let store = Track::open(&new).expect("open");
    store.record(&ran("/w", "echo learnt since", 0));
    drop(store);

    assert_eq!(
        from_flat_layout_in(Some(&data), None),
        Brought::default(),
        "nothing to do the second time"
    );
    let store = Track::open(&new).expect("open");
    assert!(
        store
            .commands(10)
            .iter()
            .any(|c| c.line == "echo learnt since"),
        "the second run did not overwrite what the first one produced"
    );
}

#[test]
fn every_profile_moves_not_just_the_current_one() {
    let temp = tempfile::tempdir().expect("tempdir");
    let data = data_home(temp.path());
    let root = temp.path().join("oslo");
    old_profile(&root, "default", "echo a");
    old_profile(&root, "claude", "echo b");
    old_profile(&root, "codex", "echo c");

    let brought = from_flat_layout_in(Some(&data), None);
    assert_eq!(brought.moved, ["claude", "codex", "default"]);
}

/// A profile with no model is not a failed migration — one is learnt again from the store.
#[test]
fn a_profile_without_a_model_still_moves() {
    let temp = tempfile::tempdir().expect("tempdir");
    let data = data_home(temp.path());
    let root = temp.path().join("oslo");
    old_profile(&root, "bare", "echo a");
    std::fs::remove_file(root.join("bare.model")).expect("remove");

    assert_eq!(from_flat_layout_in(Some(&data), None).moved, ["bare"]);
    let new = root.join("history/bare");
    assert!(new.join("hist.db").is_file());
    assert!(!new.join("hist.model").exists());
}

/// Nothing at all is not an error: a first-ever run has no old layout to bring forward.
#[test]
fn an_empty_data_directory_moves_nothing() {
    let temp = tempfile::tempdir().expect("tempdir");
    let data = data_home(temp.path());
    assert_eq!(from_flat_layout_in(Some(&data), None), Brought::default());
    assert_eq!(
        from_flat_layout_in(None, None),
        Brought::default(),
        "and nowhere to look"
    );
}

/// **A store that will not open is reported, not skipped.** Both this machine's `claude` and `codex`
/// stores turned out to be unreadable, and a migration that stayed quiet about them would leave two
/// profiles looking empty in the new layout with nothing to say why.
#[test]
fn a_store_that_cannot_be_read_is_reported_and_leaves_nothing_behind() {
    let temp = tempfile::tempdir().expect("tempdir");
    let data = data_home(temp.path());
    let root = temp.path().join("oslo");
    old_profile(&root, "fine", "echo a");
    std::fs::write(root.join("broken.kv"), b"not a database at all").expect("write");

    let brought = from_flat_layout_in(Some(&data), None);
    assert_eq!(brought.moved, ["fine"]);
    assert_eq!(brought.failed, ["broken"], "and it says which");

    // No half-made profile left over: `available()` must not offer one that holds nothing.
    assert!(!root.join("history/broken").exists());
    assert!(
        root.join("broken.kv").is_file(),
        "the original is untouched"
    );
}

/// The other files in `<data>/oslo/` are not profiles. A plugin's database is a `.kv` too, and it
/// lives one directory down — but the sibling `universal` and `direnv/` are right here.
#[test]
fn only_profile_stores_are_taken_for_profiles() {
    let temp = tempfile::tempdir().expect("tempdir");
    let data = data_home(temp.path());
    let root = temp.path().join("oslo");
    old_profile(&root, "real", "echo a");
    std::fs::write(root.join("universal"), b"not a profile").expect("write");
    std::fs::create_dir_all(root.join("direnv/allow")).expect("dir");
    std::fs::create_dir_all(root.join("plugins")).expect("dir");
    std::fs::write(root.join("plugins/notes.kv"), b"a plugin's own").expect("write");
    // A name a profile could never have had.
    std::fs::write(root.join("9nope.kv"), b"x").expect("write");

    assert_eq!(from_flat_layout_in(Some(&data), None).moved, ["real"]);
}

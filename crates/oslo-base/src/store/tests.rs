//! **These never touch `$XDG_DATA_HOME`.** Every test that opens something opens it under a
//! temporary directory by path, so the suite cannot write into the user's real plugin databases —
//! and [`open`] itself, which is the only thing that consults the environment, is exercised through
//! [`path_of`] rather than by moving the process's environment out from under its siblings.

use super::*;

fn temp_store() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = Store::open(&dir.path().join("t.kv")).expect("open");
    (dir, store)
}

#[test]
fn a_value_survives_a_round_trip_exactly() {
    let (_dir, store) = temp_store();
    // Bytes, not text: a value is whatever Lua handed over, including a NUL and a newline.
    let value = b"one\ntwo\0three".to_vec();
    set(&store, "k", &value).expect("set");
    assert_eq!(get(&store, "k"), Some(value));
}

#[test]
fn an_empty_value_is_present_rather_than_absent() {
    let (_dir, store) = temp_store();
    set(&store, "k", b"").expect("set");
    assert_eq!(get(&store, "k"), Some(Vec::new()));
    assert!(has(&store, "k"), "an empty value is still a key");
    assert!(!has(&store, "missing"));
}

#[test]
fn deleting_says_whether_there_was_anything_to_delete() {
    let (_dir, store) = temp_store();
    set(&store, "k", b"v").expect("set");
    assert!(delete(&store, "k"));
    assert!(!delete(&store, "k"), "the second time there is nothing");
    assert_eq!(get(&store, "k"), None);
}

#[test]
fn keys_come_back_in_order_and_only_under_the_prefix() {
    let (_dir, store) = temp_store();
    for key in ["b/2", "a/1", "b/1", "c"] {
        set(&store, key, b"v").expect("set");
    }
    assert_eq!(keys(&store, "b/"), vec!["b/1", "b/2"]);
    assert_eq!(keys(&store, ""), vec!["a/1", "b/1", "b/2", "c"]);
    assert!(keys(&store, "zz").is_empty());
}

#[test]
fn an_oversized_value_is_refused_before_it_is_written() {
    let (_dir, store) = temp_store();
    let huge = vec![b'x'; MAX_VALUE + 1];
    let refused = set(&store, "k", &huge).expect_err("should refuse");
    assert!(refused.contains("limit"), "{refused}");
    assert_eq!(get(&store, "k"), None, "nothing may have been written");
}

#[test]
fn an_empty_or_oversized_key_is_refused() {
    let (_dir, store) = temp_store();
    assert!(set(&store, "", b"v").is_err());
    assert!(set(&store, &"k".repeat(MAX_KEY + 1), b"v").is_err());
}

/// **The check that keeps one caller out of another's data**, and out of everything else.
#[test]
fn a_name_that_could_name_another_file_is_refused() {
    for bad in [
        "../history",    // up and out
        "a/b",           // into a subdirectory
        "/etc/shadow",   // absolute
        ".hidden",       // a leading dot is not a name here
        "",              // nothing
        "with space",    // not a filename component we will construct
        "wîth-unicode",  // ASCII only, so the byte length is the length
        &"n".repeat(65), // too long
    ] {
        assert!(!valid_name(bad), "{bad:?} was accepted");
        assert!(path_of(bad).is_none(), "{bad:?} produced a path");
    }
    for good in ["notes", "my.plugin", "a_b-c", "x1"] {
        assert!(valid_name(good), "{good:?} was refused");
    }
}

/// A valid name lands in the plugin directory and nowhere else.
#[test]
fn a_name_maps_into_the_plugin_directory() {
    let Some(directory) = directory() else {
        return; // no HOME and no XDG_DATA_HOME in this environment; nothing to assert
    };
    let path = path_of("notes").expect("a path");
    assert_eq!(path.parent(), Some(directory.as_path()));
    assert_eq!(path.file_name().unwrap(), "notes.kv");
}

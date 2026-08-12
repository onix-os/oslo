use super::*;

fn plugin_with(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("temp dir");
    for (name, body) in files {
        let path = dir.path().join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("mkdir");
        }
        std::fs::write(path, body).expect("write");
    }
    dir
}

#[test]
fn the_same_files_hash_the_same_and_a_changed_one_does_not() {
    let dir = plugin_with(&[("plugin.lua", "return {}"), ("init.lua", "-- one")]);
    let first = hash_of(dir.path()).expect("hash");
    assert_eq!(first, hash_of(dir.path()).expect("hash"), "not stable");
    assert!(unchanged(dir.path(), &first).expect("compare"));

    std::fs::write(dir.path().join("init.lua"), "-- two").expect("write");
    assert_ne!(first, hash_of(dir.path()).expect("hash"));
    assert!(!unchanged(dir.path(), &first).expect("compare"));
}

/// A file appearing is a change, and so is one going away.
#[test]
fn adding_or_removing_lua_changes_the_hash() {
    let dir = plugin_with(&[("plugin.lua", "return {}"), ("init.lua", "-- one")]);
    let before = hash_of(dir.path()).expect("hash");

    std::fs::write(dir.path().join("extra.lua"), "-- more").expect("write");
    let with_extra = hash_of(dir.path()).expect("hash");
    assert_ne!(before, with_extra);

    std::fs::remove_file(dir.path().join("extra.lua")).expect("remove");
    assert_eq!(before, hash_of(dir.path()).expect("hash"));
}

/// **Renaming a file is a change**, which is why the path is hashed and not only the contents.
#[test]
fn moving_the_same_bytes_to_another_name_changes_the_hash() {
    let dir = plugin_with(&[("plugin.lua", "return {}"), ("a.lua", "-- body")]);
    let before = hash_of(dir.path()).expect("hash");
    std::fs::rename(dir.path().join("a.lua"), dir.path().join("b.lua")).expect("rename");
    assert_ne!(before, hash_of(dir.path()).expect("hash"));
}

/// Only Lua is loaded, so only Lua decides whether the plugin changed.
#[test]
fn documentation_is_not_part_of_the_hash() {
    let dir = plugin_with(&[("plugin.lua", "return {}"), ("README.md", "before")]);
    let before = hash_of(dir.path()).expect("hash");
    std::fs::write(dir.path().join("README.md"), "after, and much longer").expect("write");
    assert_eq!(
        before,
        hash_of(dir.path()).expect("hash"),
        "editing a README must not stop a plugin loading"
    );
}

#[test]
fn nested_lua_counts_and_a_directory_with_none_is_refused() {
    let dir = plugin_with(&[("plugin.lua", "return {}"), ("lib/helper.lua", "-- helper")]);
    let before = hash_of(dir.path()).expect("hash");
    std::fs::write(dir.path().join("lib/helper.lua"), "-- changed").expect("write");
    assert_ne!(before, hash_of(dir.path()).expect("hash"));

    let empty = tempfile::tempdir().expect("temp dir");
    assert!(hash_of(empty.path()).is_err());
}

/// A link out of the plugin is content the install never saw.
#[test]
fn a_symlink_is_not_hashed() {
    let dir = plugin_with(&[("plugin.lua", "return {}")]);
    let outside = tempfile::tempdir().expect("temp dir");
    let target = outside.path().join("elsewhere.lua");
    std::fs::write(&target, "-- not mine").expect("write");

    let before = hash_of(dir.path()).expect("hash");
    std::os::unix::fs::symlink(&target, dir.path().join("linked.lua")).expect("symlink");
    assert_eq!(
        before,
        hash_of(dir.path()).expect("hash"),
        "a symlink must not join the hash"
    );
}

//! The parts of the path that are easy to get wrong and impossible to notice when they are.
//!
//! These drive `plugin_files` against a tree on disk rather than the real roots: `roots()` reads
//! `$HOME` and the XDG variables, and a test that set them would race every other test in the
//! process.

use super::*;

fn touch(path: &Path) {
    std::fs::create_dir_all(path.parent().expect("a parent")).expect("mkdir");
    std::fs::write(path, "").expect("write");
}

#[test]
fn files_are_sorted_and_a_subdirectory_follows_its_parent() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path().to_path_buf();
    touch(&root.join("plugin/b.lua"));
    touch(&root.join("plugin/a.lua"));
    touch(&root.join("plugin/skip.txt"));
    touch(&root.join("plugin/sub/c.lua"));
    // `lua/` is for requiring, never for running: a file here must not appear at all.
    touch(&root.join("lua/helper.lua"));

    let files = plugin_files(&[Root {
        path: root.clone(),
        after: false,
    }]);
    let names: Vec<String> = files.iter().map(PluginFile::label).collect();
    assert_eq!(names, ["plugin/a.lua", "plugin/b.lua", "plugin/sub/c.lua"]);
    for file in &files {
        assert_eq!(file.root, root, "each file knows the root it came from");
    }
}

#[test]
fn a_symlinked_plugin_is_walked_like_a_real_one() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let root = tmp.path().join("root");
    touch(&tmp.path().join("elsewhere/dev.lua"));
    std::fs::create_dir_all(root.join("plugin")).expect("mkdir");
    // Developing a plugin means linking it in rather than copying it on every save.
    if std::os::unix::fs::symlink(
        tmp.path().join("elsewhere/dev.lua"),
        root.join("plugin/dev.lua"),
    )
    .is_err()
    {
        return;
    }

    let files = plugin_files(&[Root {
        path: root,
        after: false,
    }]);
    assert_eq!(
        files.iter().map(PluginFile::label).collect::<Vec<_>>(),
        ["plugin/dev.lua"]
    );
}

#[test]
fn the_after_half_mirrors_the_head_so_yours_is_first_and_last() {
    let list = roots();
    assert!(list.len() >= 4);
    assert_eq!(list.len() % 2, 0);
    assert!(!list[0].after);

    // The last root is the first one's `after`: whatever you put in your own config directory gets
    // the final word over everything oslo ships.
    let last = list.last().expect("a last root");
    assert!(last.after);
    assert_eq!(last.path, list[0].path.join(AFTER_DIR));

    // Once the after half starts, no plain root may follow it.
    let mut seen_after = false;
    for root in &list {
        if root.after {
            seen_after = true;
        } else {
            assert!(!seen_after, "a plain root after an `after` root");
        }
    }
}

#[test]
fn every_root_contributes_its_lua_directory_to_the_require_path() {
    let path = require_path();
    for root in roots() {
        let want = format!("{}/{LUA_DIR}/?.lua", root.path.display());
        assert!(path.contains(&want), "missing {want}");
    }
    // Never a trailing `;`: Lua reads an empty template as the bare name and tries to load it.
    assert!(!path.ends_with(';'));
}

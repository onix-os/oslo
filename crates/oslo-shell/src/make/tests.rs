//! Discovery, asserted against a real tree rather than a mocked one — it is three `stat`s.

use super::*;

/// A scratch directory that takes itself away.
struct Tree(PathBuf);

impl Tree {
    fn new(tag: &str) -> Tree {
        let root = std::env::temp_dir().join(format!("oslo-make-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("scratch directory");
        Tree(root)
    }

    fn dir(&self, rel: &str) -> PathBuf {
        let path = self.0.join(rel);
        std::fs::create_dir_all(&path).expect("directory");
        path
    }

    fn file(&self, rel: &str) -> PathBuf {
        let path = self.0.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("parent");
        }
        std::fs::write(&path, "-- recipes\n").expect("file");
        path
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn a_directory_with_the_file_answers_with_its_own() {
    let tree = Tree::new("own");
    let file = tree.file(NAME);
    assert_eq!(governing(&tree.0), Some(file));
}

/// The walk goes up, so a subdirectory is governed by the project above it.
#[test]
fn a_subdirectory_is_governed_by_the_nearest_ancestor() {
    let tree = Tree::new("up");
    let file = tree.file(NAME);
    let deep = tree.dir("a/b/c");
    assert_eq!(governing(&deep), Some(file));
}

/// **Nearest wins outright.** Loading both would make the outer file's effect depend on how deep
/// you were standing, which is not something a reader can see in either file.
#[test]
fn the_inner_file_shadows_the_outer_one() {
    let tree = Tree::new("shadow");
    tree.file(NAME);
    let inner = tree.file(&format!("app/{NAME}"));
    assert_eq!(governing(&tree.0.join("app")), Some(inner));
}

#[test]
fn a_tree_with_no_file_answers_nothing() {
    let tree = Tree::new("none");
    let deep = tree.dir("x/y");
    assert_eq!(governing(&deep), None);
}

/// A directory of that name is not a file of that name, and running it would be a strange error.
#[test]
fn a_directory_by_that_name_is_not_a_recipe_file() {
    let tree = Tree::new("dir");
    tree.dir(NAME);
    assert_eq!(governing(&tree.0), None);
}

#[test]
fn the_root_is_the_directory_holding_the_file() {
    let tree = Tree::new("root");
    let file = tree.file(&format!("app/{NAME}"));
    assert_eq!(root_of(&file), Some(tree.0.join("app").as_path()));
}

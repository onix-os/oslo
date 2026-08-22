//! Which file governs a directory.
//!
//! Walk **up** from the directory to the root and take the nearest ancestor that has one — direnv's
//! rule. Nearest, not all: `~/work/.env.lua` and `~/work/app/.env.lua` both existing means the inner
//! one applies and the outer one does not. Loading every ancestor would make the outer file's effect
//! depend on how deep you happened to be standing, which is not something you can read off the file.
//!
//! # One name
//!
//! `.env.lua`, and nothing else. oslo used to read `.envrc` as well, against a reimplementation of
//! direnv's stdlib — 1100 lines tracking someone else's 1.4k lines of bash, so that `use flake` and
//! `layout python` meant here what they mean there.
//!
//! That is direnv's job and direnv does it. An `.envrc` project works by running direnv, which is a
//! shell line:
//!
//! ```sh
//! export PROMPT_COMMAND='eval "$(direnv export bash)"'
//! ```
//!
//! `$PROMPT_COMMAND` is evaluated against the live environment before every prompt, which is
//! exactly what direnv's own bash hook needs — load *and* unload, no oslo code in the middle. Pair
//! it with `oslo.command.when` and each tool takes the directories that are its own.

use std::path::{Path, PathBuf};

/// oslo's directory environment.
pub const NAME: &str = ".env.lua";

/// The file that governs a directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rc {
    pub path: PathBuf,
}

/// The file governing `dir`, from the nearest ancestor that has one.
///
/// `None` for almost every directory on the machine, so the cost that matters is the miss: one
/// `stat` per ancestor and nothing else.
pub fn applicable(dir: &Path) -> Option<Rc> {
    dir.ancestors().find_map(here)
}

/// The file in `dir` itself, with no walk up.
///
/// For the inherited record, which names the directory that owns the environment and so has already
/// had the walk done for it.
pub fn here(dir: &Path) -> Option<Rc> {
    let path = dir.join(NAME);
    path.is_file().then_some(Rc { path })
}

/// The directory the file lives in, for reporting and for the reload check.
pub fn owner(rc: &Rc) -> Option<PathBuf> {
    rc.path.parent().map(Path::to_path_buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, b"").expect("write");
        path
    }

    /// The nearest ancestor answers, and the ones above it do not.
    #[test]
    fn the_nearest_ancestor_wins_outright() {
        let root = tempfile::tempdir().expect("temp dir");
        let outer = root.path();
        let inner = outer.join("app/src");
        std::fs::create_dir_all(&inner).expect("mkdir");

        touch(outer, NAME);
        let near = touch(&outer.join("app"), NAME);

        let found = applicable(&inner).expect("the inner file");
        assert_eq!(found.path, near, "the outer file must not also load");
    }

    /// Nothing in `tree` answered, whatever the machine happens to have above it.
    ///
    /// **The walk has no floor, and neither does direnv's.** A test that asserted `None` outright
    /// was really asserting something about `/tmp` on the machine running it, and started failing
    /// the day a stray file appeared there — which is exactly the behaviour being relied on, found
    /// by accident.
    fn nothing_inside(tree: &Path, from: &Path) {
        match applicable(from) {
            None => {}
            Some(found) => assert!(
                !found.path.starts_with(tree),
                "nothing in the tree should answer, but {} did",
                found.path.display()
            ),
        }
    }

    /// The ordinary case: no file anywhere in the tree, answered with one `stat` per ancestor.
    #[test]
    fn a_tree_with_no_file_answers_nothing() {
        let root = tempfile::tempdir().expect("temp dir");
        let deep = root.path().join("a/b/c");
        std::fs::create_dir_all(&deep).expect("mkdir");
        nothing_inside(root.path(), &deep);
    }

    /// A *directory* called `.env.lua` is not a directory environment.
    #[test]
    fn only_regular_files_count() {
        let root = tempfile::tempdir().expect("temp dir");
        std::fs::create_dir(root.path().join(NAME)).expect("mkdir");
        nothing_inside(root.path(), root.path());
    }

    /// **An `.envrc` is not oslo's.** A directory holding one and nothing else is a directory oslo
    /// has no opinion about, which is what leaves it to direnv.
    #[test]
    fn an_envrc_is_not_a_directory_environment() {
        let root = tempfile::tempdir().expect("temp dir");
        touch(root.path(), ".envrc");
        nothing_inside(root.path(), root.path());
    }

    /// And the two coexist: a repository with both gets oslo's from oslo and direnv's from direnv,
    /// with neither shadowing the other.
    #[test]
    fn a_directory_with_both_still_answers_with_the_lua_one() {
        let root = tempfile::tempdir().expect("temp dir");
        let lua = touch(root.path(), NAME);
        touch(root.path(), ".envrc");
        assert_eq!(applicable(root.path()).expect("a file").path, lua);
    }
}

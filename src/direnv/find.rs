//! Which file governs a directory.
//!
//! Walk **up** from the directory to the root and take the nearest ancestor that has one — direnv's
//! rule. Nearest, not all: `~/work/.env.lua` and `~/work/app/.env.lua` both existing means the inner
//! one applies and the outer one does not. Loading every ancestor would make the outer file's effect
//! depend on how deep you happened to be standing, which is not something you can read off the file.
//!
//! There is exactly one name. `.envrc` and `.env` were both read here once, and both are gone:
//! `.envrc` is shell, which meant either shipping direnv's 1.4k-line stdlib or failing on every
//! real-world file that calls `use flake`; `.env` is a second grammar for something `.env.lua` says
//! in one line. Configuration is Lua, and a directory's environment is configuration.

use std::path::{Path, PathBuf};

/// The one file a directory may have.
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
    dir.ancestors()
        .map(|ancestor| ancestor.join(NAME))
        .find(|path| path.is_file())
        .map(|path| Rc { path })
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

    /// The ordinary case: no file anywhere above, answered with one `stat` per ancestor.
    #[test]
    fn a_tree_with_no_file_answers_nothing() {
        let root = tempfile::tempdir().expect("temp dir");
        let deep = root.path().join("a/b/c");
        std::fs::create_dir_all(&deep).expect("mkdir");
        assert!(applicable(&deep).is_none());
    }

    /// A *directory* called `.env.lua` is not a directory environment.
    #[test]
    fn only_regular_files_count() {
        let root = tempfile::tempdir().expect("temp dir");
        std::fs::create_dir(root.path().join(NAME)).expect("mkdir");
        assert!(applicable(root.path()).is_none());
    }
}

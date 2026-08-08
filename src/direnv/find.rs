//! Which file governs a directory.
//!
//! Walk **up** from the directory to the root and take the nearest ancestor that has one — direnv's
//! rule. Nearest, not all: `~/work/.env.lua` and `~/work/app/.env.lua` both existing means the inner
//! one applies and the outer one does not. Loading every ancestor would make the outer file's effect
//! depend on how deep you happened to be standing, which is not something you can read off the file.
//!
//! There are two names. `.env.lua` is oslo's own and is configuration in the language the rest of
//! the configuration is written in. `.envrc` is direnv's, and it is here because a shell that
//! refuses the file every other tool in the ecosystem already wrote is not a shell anyone can move
//! to — a project's `.envrc` is checked in, shared, and not yours to convert. It is shell, and oslo
//! *is* the shell, so it runs in-process against [`super::stdlib`] rather than through a subprocess.
//!
//! A directory with both is governed by `.env.lua`. Not because the shell prefers Lua, but because
//! a repository that has both almost always has the `.envrc` for everyone else and the `.env.lua`
//! for the person who wrote it, and running both would apply the same environment twice.

use std::path::{Path, PathBuf};

/// oslo's own directory environment.
pub const NAME: &str = ".env.lua";

/// direnv's, read by every other tool that does this.
pub const ENVRC: &str = ".envrc";

/// The names a directory is searched for, in the order they win.
pub const NAMES: [&str; 2] = [NAME, ENVRC];

/// What language the file is in, which is to say what runs it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// `.env.lua`, run by the Lua engine.
    Lua,
    /// `.envrc`, run by the shell with direnv's stdlib in scope.
    Shell,
}

/// The file that governs a directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rc {
    pub path: PathBuf,
    pub kind: Kind,
}

impl Rc {
    /// The file `path` would be, if it is one of the two names.
    fn at(path: PathBuf) -> Option<Rc> {
        let kind = match path.file_name()?.to_str()? {
            NAME => Kind::Lua,
            ENVRC => Kind::Shell,
            _ => return None,
        };
        Some(Rc { path, kind })
    }
}

/// The file governing `dir`, from the nearest ancestor that has one.
///
/// `None` for almost every directory on the machine, so the cost that matters is the miss: two
/// `stat`s per ancestor and nothing else.
pub fn applicable(dir: &Path) -> Option<Rc> {
    dir.ancestors().find_map(here)
}

/// The file in `dir` itself, with no walk up.
///
/// For the inherited record, which names the directory that owns the environment and so has already
/// had the walk done for it.
pub fn here(dir: &Path) -> Option<Rc> {
    NAMES
        .iter()
        .map(|name| dir.join(name))
        .find(|path| path.is_file())
        .and_then(Rc::at)
}

/// The directory the file lives in, for reporting and for the reload check.
pub fn owner(rc: &Rc) -> Option<PathBuf> {
    rc.path.parent().map(Path::to_path_buf)
}

/// The file that governs `path`'s directory, when that is not `path` itself.
///
/// **What makes the shadowed file visible.** A directory with both names has one that applies and
/// one that is inert, and being inert looks exactly like working until you notice nothing it sets
/// is set. Nothing about it can be seen from outside — it is simply never read — so `direnv status`
/// says so, and `direnv allow` refuses to write a record for a file that can never load rather than
/// accepting it and leaving you to wonder.
///
/// `None` for anything that is not one of the two names: allowing some other path is already
/// meaningless, and answering here would be this function inventing an opinion about it.
pub fn governed_by(path: &Path) -> Option<PathBuf> {
    let name = path.file_name()?.to_str()?;
    if !NAMES.contains(&name) {
        return None;
    }
    let governing = here(path.parent()?)?;
    (governing.path != path).then_some(governing.path)
}

/// The files in `dir` that are there but do not govern it.
pub fn shadowed(dir: &Path) -> Vec<PathBuf> {
    let Some(governing) = here(dir) else {
        return Vec::new();
    };
    NAMES
        .iter()
        .map(|name| dir.join(name))
        .filter(|path| *path != governing.path && path.is_file())
        .collect()
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
    /// the day a stray `/tmp/.envrc` appeared there — which is exactly the behaviour being relied
    /// on, found by accident.
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

    /// The ordinary case: no file anywhere in the tree, answered with two `stat`s per ancestor.
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

    /// **A project's `.envrc` is read, and read as shell.**
    #[test]
    fn an_envrc_governs_a_directory_too() {
        let root = tempfile::tempdir().expect("temp dir");
        let path = touch(root.path(), ENVRC);
        let found = applicable(root.path()).expect("the envrc");
        assert_eq!(found.path, path);
        assert_eq!(found.kind, Kind::Shell);
    }

    /// A nearer `.envrc` beats a further `.env.lua`: the walk is by directory, not by name.
    #[test]
    fn the_nearer_file_wins_whichever_name_it_has() {
        let root = tempfile::tempdir().expect("temp dir");
        let inner = root.path().join("app");
        std::fs::create_dir_all(&inner).expect("mkdir");
        touch(root.path(), NAME);
        let near = touch(&inner, ENVRC);
        assert_eq!(applicable(&inner).expect("the inner file").path, near);
    }

    /// Both in one directory is the repository that has an `.envrc` for everyone else. Running both
    /// would apply the same environment twice.
    #[test]
    fn a_directory_with_both_is_governed_by_the_lua_one() {
        let root = tempfile::tempdir().expect("temp dir");
        let lua = touch(root.path(), NAME);
        touch(root.path(), ENVRC);
        let found = applicable(root.path()).expect("a file");
        assert_eq!(found.path, lua);
        assert_eq!(found.kind, Kind::Lua);
    }

    /// **The ignored file can be named**, so `direnv allow` can refuse it and `direnv status` can
    /// say why nothing in it took effect.
    #[test]
    fn the_shadowed_file_is_reportable() {
        let root = tempfile::tempdir().expect("temp dir");
        let lua = touch(root.path(), NAME);
        let envrc = touch(root.path(), ENVRC);

        assert_eq!(shadowed(root.path()), vec![envrc.clone()]);
        assert_eq!(governed_by(&envrc), Some(lua.clone()));
        assert_eq!(governed_by(&lua), None, "the governing file governs itself");
    }

    /// With only one of them there is nothing shadowed and nothing to say.
    #[test]
    fn a_lone_file_shadows_nothing() {
        let root = tempfile::tempdir().expect("temp dir");
        let envrc = touch(root.path(), ENVRC);
        assert!(shadowed(root.path()).is_empty());
        assert_eq!(governed_by(&envrc), None);
    }

    /// A path that is not one of the two names gets no opinion: allowing it is already meaningless,
    /// and answering would be this inventing a rule about it.
    #[test]
    fn some_other_path_is_not_shadowed_by_anything() {
        let root = tempfile::tempdir().expect("temp dir");
        touch(root.path(), NAME);
        let other = touch(root.path(), "notes.txt");
        assert_eq!(governed_by(&other), None);
    }
}

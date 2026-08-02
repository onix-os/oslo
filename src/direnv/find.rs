//! Which rc files apply to a directory.
//!
//! Walk **up** from the directory to the root and take the nearest ancestor that has any of them —
//! direnv's rule. Nearest, not all: `~/work/.envrc` and `~/work/app/.envrc` both existing means the
//! inner one applies and the outer one does not, and an `.envrc` that wants its parent's says so
//! with `source_up`. Loading every ancestor silently would make the outer file's effect depend on
//! how deep you happened to be standing.

use std::path::{Path, PathBuf};

/// The three files, in the order they load.
///
/// `.envrc` first because it is the general one, `.env` next, `.env.lua` last so that the language
/// with the most to say gets the final word.
pub const NAMES: [&str; 3] = [".envrc", ".env", ".env.lua"];

/// What kind of file this is, which decides how it is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Shell, run on oslo's own evaluator.
    Shell,
    /// `KEY=value` lines, no execution.
    Dotenv,
    /// Lua, which may set more than variables.
    Lua,
}

impl Kind {
    fn of(name: &str) -> Kind {
        match name {
            ".env" => Kind::Dotenv,
            ".env.lua" => Kind::Lua,
            _ => Kind::Shell,
        }
    }
}

/// One rc file that applies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rc {
    pub path: PathBuf,
    pub kind: Kind,
}

/// Everything that applies to `dir`, nearest ancestor only, in load order.
///
/// Empty when nothing applies, which is the overwhelmingly common case and must therefore cost no
/// more than one `stat` per name per ancestor.
pub fn applicable(dir: &Path) -> Vec<Rc> {
    for ancestor in dir.ancestors() {
        let found: Vec<Rc> = NAMES
            .iter()
            .map(|name| ancestor.join(name))
            .filter(|path| path.is_file())
            .map(|path| {
                let kind = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .map(Kind::of)
                    .unwrap_or(Kind::Shell);
                Rc { path, kind }
            })
            .collect();
        // The first ancestor with anything at all wins, even if it has only one of the three.
        if !found.is_empty() {
            return found;
        }
    }
    Vec::new()
}

/// The directory whose rc files these are, for reporting and for the reload check.
pub fn owner(rcs: &[Rc]) -> Option<PathBuf> {
    rcs.first()?.path.parent().map(Path::to_path_buf)
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

        touch(outer, ".envrc");
        let near = touch(&outer.join("app"), ".envrc");

        let found = applicable(&inner);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].path, near, "the outer .envrc must not also load");
    }

    /// All three load together when they share a directory, in a fixed order.
    #[test]
    fn the_three_files_load_in_order() {
        let root = tempfile::tempdir().expect("temp dir");
        touch(root.path(), ".env.lua");
        touch(root.path(), ".env");
        touch(root.path(), ".envrc");

        let kinds: Vec<Kind> = applicable(root.path()).iter().map(|rc| rc.kind).collect();
        assert_eq!(
            kinds,
            vec![Kind::Shell, Kind::Dotenv, Kind::Lua],
            "Lua last, so it can override what the others set"
        );
    }

    /// A directory tree with none of them is the ordinary case and answers nothing.
    #[test]
    fn a_tree_with_no_rc_file_answers_nothing() {
        let root = tempfile::tempdir().expect("temp dir");
        let deep = root.path().join("a/b/c");
        std::fs::create_dir_all(&deep).expect("mkdir");
        assert!(applicable(&deep).is_empty());
    }

    /// A directory named `.envrc` is not an rc file.
    #[test]
    fn only_regular_files_count() {
        let root = tempfile::tempdir().expect("temp dir");
        std::fs::create_dir(root.path().join(".envrc")).expect("mkdir");
        assert!(applicable(root.path()).is_empty());
    }
}

//! `oslo.mark` — the `@` builtin, reachable from a config or a `.env.lua`.
//!
//! ```lua
//! oslo.mark()                      -- this directory, under its own last component
//! oslo.mark("proj")                -- this directory, under a name you choose
//! oslo.mark("dl", "~/Downloads")   -- somewhere else
//! oslo.unmark("proj")
//! for name, path in pairs(oslo.marks()) do print(name, path) end
//! ```
//!
//! **The case this exists for is `.env.lua`.** A repository that wants to be `@app` wherever you
//! are can say so in the file it already has, and the mark is made the first time you walk in:
//!
//! ```lua
//! -- ~/work/app/.env.lua
//! oslo.mark("app")
//! ```
//!
//! **"This directory" there means the file's own, not the one you are standing in**, and the
//! difference is not pedantic: a `.env.lua` governs everything below it, so walking straight into
//! `~/work/app/src/deep` runs `~/work/app/.env.lua` with the shell three levels down. A bare
//! `oslo.mark()` that took the shell's word for it would mark `deep` and call it `app`. While one
//! is loading, the environment's own directory is what answers.
//!
//! # Why these write to the same file `@` does
//!
//! A mark is one fact with one home. A Lua-only table beside the file would give `@name` two places
//! to look and two answers when they disagreed, and `@ -l` could show only half of it. So this is
//! the builtin with a different spelling, and `oslo.dirs` — which is *declared* and replaced on
//! every start — remains the separate thing it already was.

use super::util::{ok, put, text};
use oslo_base::dirs;
use oslo_base::value::{Table, Value};
use oslo_base::value::{LuaError, LuaResult};

/// Install `oslo.mark`, `oslo.unmark` and `oslo.marks`.
pub fn install(oslo: &mut Table) {
    put(oslo, "mark", |_, args| {
        let name = match args.first() {
            None | Some(Value::Nil) => None,
            _ => Some(text(&args, 1, "oslo.mark")?),
        };
        let path = match args.get(1) {
            None | Some(Value::Nil) => here()?,
            _ => text(&args, 2, "oslo.mark")?,
        };
        let name = match name {
            Some(name) => name,
            None => basename(&path).ok_or_else(|| {
                LuaError::new(format!(
                    "oslo.mark: {path} has no last component to name it by; pass a name"
                ))
            })?,
        };
        dirs::mark(&name, &path)
            .map_err(|problem| LuaError::new(format!("oslo.mark: {problem}")))?;
        ok(Value::str(name))
    });

    put(oslo, "unmark", |_, args| {
        let name = text(&args, 1, "oslo.unmark")?;
        let had = dirs::unmark(&name)
            .map_err(|problem| LuaError::new(format!("oslo.unmark: {problem}")))?;
        ok(Value::Bool(had))
    });

    // Every name `@` reaches, declared and marked alike — the same set the builtin's `-l` lists,
    // because a config asking "what does `@x` mean" is asking the question the sigil answers.
    put(oslo, "marks", |_, _| {
        let mut table = Table::new();
        for (name, path) in dirs::named_dirs() {
            table.set(Value::str(name), Value::str(path));
        }
        ok(Value::table(table))
    });
}

/// Which directory an environment file is speaking for, while one is being run.
///
/// All of it is behind the `direnv` feature because `.env.lua` is: a build without it has no file
/// that governs a subtree, so there is never a second candidate for "this directory".
#[cfg(feature = "direnv")]
mod loading {
    thread_local! {
        /// Thread-local rather than a parameter because the chunk in between is Lua: nothing can be
        /// threaded through it, and only the read loop's thread ever runs one.
        static LOADING: std::cell::RefCell<Option<String>> =
            const { std::cell::RefCell::new(None) };
    }

    /// Say that `dir`'s environment file is running, and take it back when it stops.
    ///
    /// A guard rather than a pair of calls: a file that raises, or that never reaches its last
    /// line, must not leave every later `oslo.mark()` pointing at a directory nobody is in.
    pub struct Loading;

    impl Loading {
        pub fn directory(dir: &std::path::Path) -> Loading {
            LOADING.with(|slot| *slot.borrow_mut() = Some(dir.to_string_lossy().into_owned()));
            Loading
        }
    }

    impl Drop for Loading {
        fn drop(&mut self) {
            LOADING.with(|slot| *slot.borrow_mut() = None);
        }
    }

    /// The directory the file being run speaks for, if one is being run.
    pub fn directory() -> Option<String> {
        LOADING.with(|slot| slot.borrow().clone())
    }
}

#[cfg(feature = "direnv")]
pub use loading::Loading;

#[cfg(not(feature = "direnv"))]
mod loading {
    pub fn directory() -> Option<String> {
        None
    }
}

/// The directory `oslo.mark()` means by "this one".
///
/// The environment file's own while one is loading, and the shell's otherwise — see the note at the
/// top of the file for why those are not the same directory.
fn here() -> LuaResult<String> {
    if let Some(dir) = loading::directory() {
        return Ok(dir);
    }
    std::env::current_dir()
        .map(|path| path.to_string_lossy().into_owned())
        .map_err(|e| {
            LuaError::new(format!(
                "oslo.mark: cannot tell which directory this is: {e}"
            ))
        })
}

/// The last component, which is what a person would call this directory.
fn basename(path: &str) -> Option<String> {
    let name = path.trim_end_matches('/').rsplit('/').next()?;
    dirs::valid_mark_name(name).then(|| name.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **While an environment file runs, "this directory" is its own.** A `.env.lua` governs
    /// everything below it, so walking straight into `app/src/deep` runs `app/.env.lua` with the
    /// shell three levels down — and a bare `oslo.mark()` that took the shell's word for it would
    /// name `deep` after the project.
    #[test]
    #[cfg(feature = "direnv")]
    fn an_environment_file_marks_its_own_directory() {
        let cwd = here().expect("a working directory");
        {
            let _loading = Loading::directory(std::path::Path::new("/home/u/work/app"));
            assert_eq!(here().expect("the file's own"), "/home/u/work/app");
        }
        // **And the guard puts it back**, so a file that raises halfway cannot leave every later
        // `oslo.mark()` pointing at a directory nobody is standing in.
        assert_eq!(here().expect("a working directory"), cwd);
    }

    /// The name a bare `oslo.mark()` would choose, and the paths that have none to choose.
    #[test]
    fn a_path_is_named_by_its_last_component() {
        assert_eq!(basename("/home/u/work/app").as_deref(), Some("app"));
        assert_eq!(basename("/home/u/work/app/").as_deref(), Some("app"));
        assert_eq!(basename("/"), None, "the root has nothing to be called");
        assert_eq!(basename(""), None);
        // A component that could not be typed back as `@name` is not a name.
        assert_eq!(basename("/home/u/my work"), None);
    }
}

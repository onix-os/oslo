//! The one place the shell changes directory.
//!
//! `cd`, `pushd` and `popd` all land in [`change_directory`], so the three things that are easy
//! to get subtly wrong are decided once: which reading of a path `$PWD` keeps, that `$OLDPWD` is
//! written on *every* move (a later `cd -` is only as good as the last builtin that moved), and
//! the `CDPATH` search with its POSIX echo.
//!
//! The logical/physical distinction is the substance of the module. A shell that only ever
//! stores `getcwd()` — which is what rush did — cannot answer `cd /tmp/link; pwd` the way every
//! other shell does: the answer is the path the user walked, not the path the kernel resolved.

use crate::env::scope::Environment;
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Which reading of a path the shell keeps in `$PWD`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PathMode {
    /// `-L`, the default: symlinked components stay in `$PWD` and `..` is resolved lexically,
    /// so `cd link; cd ..` returns to where `link` was named, not to the target's parent.
    Logical,
    /// `-P`: every symlink is resolved, so `$PWD` is what `getcwd()` reports.
    Physical,
}

/// The shell's logical working directory.
///
/// `$PWD` is authoritative — that is the entire point of `-L` — but only while it still names
/// the directory the process is really in. An inherited `$PWD` from a parent that has since
/// moved (every `rush -c` the differential harness spawns inherits one) would otherwise make
/// `pwd` lie about a directory this shell never visited, so it is validated against `getcwd()`
/// before it is trusted, exactly as bash validates it at startup.
pub fn logical_pwd(env: &Environment) -> String {
    let physical = env::current_dir().unwrap_or_else(|_| PathBuf::from("/"));
    if let Some(pwd) = env.get_var("PWD")
        && Path::new(pwd).is_absolute()
        && fs::canonicalize(pwd).is_ok_and(|resolved| resolved == physical)
    {
        return pwd.to_string();
    }
    physical.to_string_lossy().into_owned()
}

/// Where an operand landed the shell.
struct Landing {
    /// The value `$PWD` takes — physical under `-P`, the logical path under `-L`.
    pwd: String,
    /// The same destination named logically. This is what `cd -` and a `CDPATH` hit echo, in
    /// either mode: bash prints the path it was *asked* for, resolved against `$PWD`, even when
    /// `-P` then stores something else.
    display: String,
}

/// Join `operand` onto `base` unless it is already absolute. `base` is always absolute.
fn absolute_against(base: &str, operand: &str) -> String {
    if operand.starts_with('/') {
        operand.to_string()
    } else {
        format!("{}/{}", base.trim_end_matches('/'), operand)
    }
}

/// Resolve `.` and `..` textually, without touching the filesystem.
///
/// This is what makes `-L` logical: `..` cancels the *name* to its left, so a symlinked
/// component is stepped back out of rather than followed. Input must be absolute.
fn lexical_canon(path: &str) -> String {
    let mut kept: Vec<&str> = Vec::new();
    for component in path.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                kept.pop();
            }
            other => kept.push(other),
        }
    }
    format!("/{}", kept.join("/"))
}

/// Name `operand` the way `-L` would, relative to `base`, without going anywhere.
///
/// `pushd -n dir` needs a stack entry for a directory the shell is not moving to, and a stack
/// full of relative paths would break the moment the shell moved.
pub fn logical_join(base: &str, operand: &str) -> String {
    lexical_canon(&absolute_against(base, operand))
}

/// Try to move to `operand`, interpreted relative to `base`. Nothing is printed and no shell
/// variable is touched; only the process's working directory changes.
fn attempt(base: &str, operand: &str, mode: PathMode) -> io::Result<Landing> {
    let absolute = absolute_against(base, operand);
    let logical = lexical_canon(&absolute);

    match mode {
        PathMode::Physical => {
            let resolved = fs::canonicalize(&absolute)?;
            env::set_current_dir(&resolved)?;
            Ok(Landing {
                pwd: resolved.to_string_lossy().into_owned(),
                display: logical,
            })
        }
        PathMode::Logical => {
            // The lexical path is only used when the path as written also exists. Cancelling
            // `..` first would make `cd nosuch/..` a silent success that leaves the shell where
            // it started, and bash rejects it for the same reason.
            let intact = fs::metadata(&absolute).is_ok_and(|meta| meta.is_dir());
            if intact {
                env::set_current_dir(&logical)?;
                Ok(Landing {
                    pwd: logical.clone(),
                    display: logical,
                })
            } else {
                env::set_current_dir(&absolute)?;
                // A path that stat() refused but chdir() accepted is too strange to describe
                // logically; fall back to what the kernel says.
                let pwd = env::current_dir()
                    .unwrap_or_else(|_| PathBuf::from(&logical))
                    .to_string_lossy()
                    .into_owned();
                Ok(Landing {
                    pwd,
                    display: logical,
                })
            }
        }
    }
}

/// Whether `CDPATH` gets a say. A path that already says where it starts from — absolute, or
/// explicitly `.`/`..`-anchored — is never searched for.
fn searches_cdpath(operand: &str) -> bool {
    !(operand.starts_with('/')
        || operand == "."
        || operand == ".."
        || operand.starts_with("./")
        || operand.starts_with("../"))
}

/// `CDPATH` split on `:`. An empty element means the current directory, and is kept because the
/// difference between an empty and a non-empty element decides whether the hit is echoed.
fn cdpath_entries(env: &Environment) -> Vec<String> {
    match env.get_var("CDPATH") {
        Some(value) if !value.is_empty() => value.split(':').map(str::to_string).collect(),
        _ => Vec::new(),
    }
}

/// Change the shell's working directory and keep `$PWD`/`$OLDPWD` in step.
///
/// Returns the destination as it should be echoed on success — callers decide whether to print
/// it — or `None` after printing a diagnostic naming `caller`. The status a builtin reports for
/// `None` is 1: the shell has not moved.
pub fn change_directory(
    env: &mut Environment,
    operand: &str,
    mode: PathMode,
    caller: &str,
) -> Option<String> {
    if operand.is_empty() {
        eprintln!("rush: {caller}: null directory");
        return None;
    }

    let origin = logical_pwd(env);

    let mut landing = None;
    if searches_cdpath(operand) {
        for entry in cdpath_entries(env) {
            let candidate = if entry.is_empty() {
                operand.to_string()
            } else {
                format!("{}/{}", entry, operand)
            };
            if let Ok(found) = attempt(&origin, &candidate, mode) {
                // POSIX: when a *non-empty* CDPATH element supplies the directory, the new
                // directory is written to stdout, so a script can tell it did not land where
                // the bare operand would have taken it. An empty element is the current
                // directory, which is where the operand pointed anyway — no surprise, no echo.
                if !entry.is_empty() {
                    println!("{}", found.display);
                }
                landing = Some(found);
                break;
            }
        }
    }

    let landing = match landing {
        Some(found) => found,
        None => match attempt(&origin, operand, mode) {
            Ok(found) => found,
            Err(e) => {
                eprintln!("rush: {caller}: {operand}: {e}");
                return None;
            }
        },
    };

    env.set_var("OLDPWD", &origin, true);
    env.set_var("PWD", &landing.pwd, true);
    Some(landing.display)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lexical_canon_cancels_names_not_targets() {
        assert_eq!(lexical_canon("/a/b/.."), "/a");
        assert_eq!(lexical_canon("/a/./b"), "/a/b");
        assert_eq!(lexical_canon("/a//b/"), "/a/b");
        assert_eq!(lexical_canon("/a/b/../.."), "/");
        // `..` at the root stays at the root, as chdir would.
        assert_eq!(lexical_canon("/../.."), "/");
    }

    #[test]
    fn absolute_against_does_not_double_the_root_slash() {
        assert_eq!(absolute_against("/", "tmp"), "/tmp");
        assert_eq!(absolute_against("/a", "b"), "/a/b");
        assert_eq!(absolute_against("/a", "/b"), "/b");
    }

    #[test]
    fn cdpath_is_skipped_for_anchored_paths() {
        assert!(searches_cdpath("sub"));
        assert!(searches_cdpath("a/b"));
        assert!(!searches_cdpath("/abs"));
        assert!(!searches_cdpath("."));
        assert!(!searches_cdpath(".."));
        assert!(!searches_cdpath("./sub"));
        assert!(!searches_cdpath("../sub"));
    }
}

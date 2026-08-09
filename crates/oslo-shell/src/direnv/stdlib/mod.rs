//! direnv's standard library, in Rust.
//!
//! An `.envrc` is shell, and almost every real one is written against the functions direnv makes
//! available to it: `PATH_add`, `use flake`, `layout python`, `dotenv`, `source_up`. A shell that
//! reads `.envrc` but not those reads almost nothing — direnv's own stdlib is some 1.4k lines of
//! bash, and refusing to ship it is why oslo used to decline the file by name.
//!
//! # Why Rust and not the bash
//!
//! direnv ships its stdlib *as bash text* because it has to: it is an external program handing a
//! script to a shell it did not write, so the only thing it can send is source. oslo is the shell.
//! Writing them here means `PATH_add` is a path operation rather than a string operation on `$PATH`,
//! `find_up` is a walk rather than a loop of `cd ..`, and none of it depends on which bashisms the
//! parser has this month. It also means the functions are testable without a subprocess.
//!
//! # Scope
//!
//! These exist while an `.envrc` is being run and nowhere else — [`install`] before, [`remove`]
//! after — which is direnv's own rule. `PATH_add` at the prompt would be a command that edits an
//! environment no file is holding open, and the undo record would never hear about it.
//!
//! # Credit
//!
//! The behaviour, the names and the semantics are direnv's, by zimbatm and contributors
//! (<https://github.com/direnv/direnv>, MIT). What is written here is a reimplementation against
//! its documented interface, so that files written for it work unchanged.

mod layout;
mod nix;
mod paths;
mod report;
mod sourcing;

#[cfg(test)]
mod tests;

use crate::env::Environment;
use crate::env::scope::BuiltinFn;
use oslo_base::error::Result;
use std::cell::RefCell;
use std::path::{Path, PathBuf};

/// Every name an `.envrc` may call, and what runs it.
///
/// One table, read by both [`install`] and [`remove`], so a function cannot be added to the shell
/// without also being taken back out of it. Two lists drifted apart once already in this codebase
/// and it cost a release; this is the cheap structural answer.
const STDLIB: &[(&str, BuiltinFn)] = &[
    ("PATH_add", paths::path_add_front),
    ("PATH_rm", paths::path_rm),
    ("MANPATH_add", paths::manpath_add),
    ("path_add", paths::var_add),
    ("path_rm", paths::var_rm),
    ("expand_path", paths::expand),
    ("find_up", paths::find_up),
    ("user_rel_path", paths::user_rel),
    ("direnv_layout_dir", paths::layout_dir),
    ("log_status", report::log_status),
    ("log_error", report::log_error),
    ("has", report::has),
    ("join_args", report::join_args),
    ("env_vars_required", report::env_vars_required),
    ("on_git_branch", report::on_git_branch),
    ("strict_env", report::strict_env),
    ("unstrict_env", report::unstrict_env),
    ("direnv_version", report::direnv_version),
    ("watch_file", report::watch_file),
    ("watch_dir", report::watch_dir),
    ("source_env", sourcing::source_env),
    ("source_env_if_exists", sourcing::source_env_if_exists),
    ("source_up", sourcing::source_up),
    ("source_up_if_exists", sourcing::source_up_if_exists),
    ("dotenv", sourcing::dotenv),
    ("dotenv_if_exists", sourcing::dotenv_if_exists),
    ("use", nix::use_dispatch),
    ("use_flake", nix::use_flake),
    ("use_nix", nix::use_nix),
    ("layout", layout::dispatch),
];

/// Put the stdlib in scope.
pub fn install(env: &mut Environment) {
    for (name, func) in STDLIB {
        env.register_custom_builtin(name, *func);
    }
}

/// Take it back out.
pub fn remove(env: &mut Environment) {
    for (name, _) in STDLIB {
        env.unregister_custom_builtin(name);
    }
}

thread_local! {
    /// Files this run asked to be watched, beyond the `.envrc` itself.
    ///
    /// Thread-local rather than threaded through every function because the callers are shell
    /// builtins, whose signature is fixed by the registry and has nowhere to put it. Drained by
    /// [`take_watches`] when the run ends, so one directory's watches cannot become another's.
    static WATCHED: RefCell<Vec<PathBuf>> = const { RefCell::new(Vec::new()) };
}

/// Record a file whose edit should reload the environment.
pub fn watch(path: &Path) {
    let path = path.to_path_buf();
    WATCHED.with(|watched| {
        let mut watched = watched.borrow_mut();
        if !watched.contains(&path) {
            watched.push(path);
        }
    });
}

/// Everything [`watch`] collected, clearing it.
pub fn take_watches() -> Vec<PathBuf> {
    WATCHED.with(|watched| std::mem::take(&mut *watched.borrow_mut()))
}

/// The directory the shell is standing in.
///
/// The process's, not a variable's: `$PWD` is what the shell *published* and an `.envrc` is free to
/// have overwritten it, while every path this stdlib resolves has to be relative to where the file
/// actually is.
pub(crate) fn here() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

/// `path` made absolute against `base`, with `~` expanded, and no filesystem access.
///
/// Lexical on purpose: `expand_path` is documented to work on paths that do not exist yet, which is
/// most of what a layout function builds.
pub(crate) fn absolute(path: &str, base: &Path) -> PathBuf {
    let expanded = match path.strip_prefix("~") {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => match std::env::var("HOME") {
            Ok(home) => PathBuf::from(home).join(rest.trim_start_matches('/')),
            Err(_) => PathBuf::from(path),
        },
        _ => PathBuf::from(path),
    };
    if expanded.is_absolute() {
        return tidy(&expanded);
    }
    tidy(&base.join(expanded))
}

/// `.` and `..` resolved textually, so a path that does not exist still comes out clean.
fn tidy(path: &Path) -> PathBuf {
    let mut parts: Vec<std::ffi::OsString> = Vec::new();
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if parts.pop().is_none() {
                    out.push("..");
                }
            }
            std::path::Component::Normal(name) => parts.push(name.to_os_string()),
            other => out.push(other.as_os_str()),
        }
    }
    for part in parts {
        out.push(part);
    }
    out
}

pub use paths::prepend_into;

/// Run a word list as a command, through the shell rather than around it.
///
/// `eval` of a quoted line, so a shell function, an alias and an external command are all reached
/// the way the shell would reach them. A stdlib function that resolved commands itself would be a
/// second, slightly different command search living next to the real one.
pub(crate) fn run(env: &mut Environment, words: &[String]) -> Result<i32> {
    let line: Vec<String> = words.iter().map(|word| quote(word)).collect();
    crate::env::builtins::builtin_eval(env, &["eval".to_string(), line.join(" ")])
}

/// Single quotes unless the word plainly does not need them.
pub(crate) fn quote(word: &str) -> String {
    let plain = !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_-./:=@+".contains(c));
    if plain {
        return word.to_string();
    }
    format!("'{}'", word.replace('\'', r"'\''"))
}

/// Complain the way direnv does, on stderr with the file's name in front of it.
pub(crate) fn fault(function: &str, message: &str) -> oslo_base::error::Result<i32> {
    eprintln!("direnv: {function}: {message}");
    Ok(1)
}

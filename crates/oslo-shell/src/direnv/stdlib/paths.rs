//! The path functions: `PATH_add` and the family around it.
//!
//! These are the reason a stdlib written in Rust is better than the bash one rather than merely
//! equivalent. direnv's `PATH_add` is string surgery on `$PATH` — it has to be, having only a shell
//! to work with — and every one of its edge cases is a string edge case: a trailing colon, an entry
//! that is already there, a relative directory that means something different once you have moved.
//! Here they are path operations, and the edge cases stop existing.

use super::{absolute, fault, here};
use crate::env::Environment;
use crate::env::lists;
use oslo_base::error::Result;
use std::path::{Path, PathBuf};

/// `PATH_add <dir>...` — prepend to `$PATH`.
pub fn path_add_front(env: &mut Environment, args: &[String]) -> Result<i32> {
    prepend(env, "PATH", &args[1..], "PATH_add")
}

/// `MANPATH_add <dir>...`
pub fn manpath_add(env: &mut Environment, args: &[String]) -> Result<i32> {
    prepend(env, "MANPATH", &args[1..], "MANPATH_add")
}

/// `path_add <VARNAME> <dir>...` — the same, for any colon-separated variable.
pub fn var_add(env: &mut Environment, args: &[String]) -> Result<i32> {
    let Some(name) = args.get(1) else {
        return fault("path_add", "needs a variable name");
    };
    prepend(env, name, &args[2..], "path_add")
}

/// `PATH_rm <pattern>...` — drop entries matching a shell pattern.
pub fn path_rm(env: &mut Environment, args: &[String]) -> Result<i32> {
    remove(env, "PATH", &args[1..])
}

/// `path_rm <VARNAME> <pattern>...`
pub fn var_rm(env: &mut Environment, args: &[String]) -> Result<i32> {
    let Some(name) = args.get(1) else {
        return fault("path_rm", "needs a variable name");
    };
    remove(env, name, &args[2..])
}

/// `PATH_add` for callers that are not builtins — the layouts, and the Lua API.
pub fn prepend_into(env: &mut Environment, name: &str, dirs: &[String]) -> Result<i32> {
    prepend(env, name, dirs, "path_add")
}

/// Put `dirs` at the front of `name`, absolute, in the order given, each appearing once.
///
/// **Idempotent, which is the whole point.** A directory environment is loaded and reloaded — on an
/// edit, on a nested shell, on `direnv allow` — and a `PATH_add` that appended unconditionally would
/// grow `$PATH` by one entry every time until it was pages long. direnv gets this right and so must
/// this: an entry already present is moved to the front rather than added again.
fn prepend(env: &mut Environment, name: &str, dirs: &[String], function: &str) -> Result<i32> {
    if dirs.is_empty() {
        return fault(function, "needs a directory");
    }
    lists::prepend(env, name, dirs, &here());
    Ok(0)
}

/// Drop every entry of `name` matching any of `patterns`.
fn remove(env: &mut Environment, name: &str, patterns: &[String]) -> Result<i32> {
    lists::remove(env, name, patterns);
    Ok(0)
}

/// `expand_path <path> [base]` — print `path` absolute, without touching the disk.
pub fn expand(_env: &mut Environment, args: &[String]) -> Result<i32> {
    let Some(path) = args.get(1) else {
        return fault("expand_path", "needs a path");
    };
    let base = match args.get(2) {
        Some(base) => absolute(base, &here()),
        None => here(),
    };
    println!("{}", absolute(path, &base).display());
    Ok(0)
}

/// `find_up <name>` — print the nearest ancestor holding `name`, or nothing.
///
/// Answers with the *file*, as direnv does, so `source_env "$(find_up .envrc)"` works.
pub fn find_up(_env: &mut Environment, args: &[String]) -> Result<i32> {
    let Some(name) = args.get(1) else {
        return fault("find_up", "needs a name");
    };
    match upwards(&here(), name) {
        Some(found) => {
            println!("{}", found.display());
            Ok(0)
        }
        None => Ok(1),
    }
}

/// The nearest `name` at or above `from`.
pub(crate) fn upwards(from: &Path, name: &str) -> Option<PathBuf> {
    from.ancestors()
        .map(|ancestor| ancestor.join(name))
        .find(|candidate| candidate.exists())
}

/// `user_rel_path <path>` — print it with `$HOME` written as `~`.
pub fn user_rel(_env: &mut Environment, args: &[String]) -> Result<i32> {
    let Some(path) = args.get(1) else {
        println!();
        return Ok(0);
    };
    let shown = match std::env::var("HOME") {
        Ok(home) if !home.is_empty() => match path.strip_prefix(&home) {
            Some("") => "~".to_string(),
            Some(rest) if rest.starts_with('/') => format!("~{rest}"),
            _ => path.clone(),
        },
        _ => path.clone(),
    };
    println!("{shown}");
    Ok(0)
}

/// `direnv_layout_dir` — where a layout may keep what it builds.
///
/// `$direnv_layout_dir` if the file set one, so a project can move it out of the tree; otherwise
/// `.direnv` beside the `.envrc`, which is what every layout function and every `.gitignore` in the
/// wild expects.
pub fn layout_dir(env: &mut Environment, _args: &[String]) -> Result<i32> {
    let chosen = env
        .get_var("direnv_layout_dir")
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    match chosen {
        Some(dir) => println!("{dir}"),
        None => println!("{}", here().join(".direnv").display()),
    }
    Ok(0)
}

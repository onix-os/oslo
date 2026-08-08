//! Pulling in another file: `source_env`, `source_up`, `dotenv`.
//!
//! Each of these adds a file the environment now depends on, so each one calls [`super::watch`].
//! Forgetting that is the classic direnv bug report — a project whose `.env` is edited and whose
//! shell goes on holding the old values until something unrelated forces a reload.

use super::paths::upwards;
use super::{absolute, fault, here, watch};
use crate::env::Environment;
use crate::error::Result;
use std::path::{Path, PathBuf};

/// `source_env <file-or-dir>` — run another rc file in this environment.
pub fn source_env(env: &mut Environment, args: &[String]) -> Result<i32> {
    let Some(target) = args.get(1) else {
        return fault("source_env", "needs a path");
    };
    match resolve(target) {
        Some(path) => run(env, &path),
        None => fault("source_env", &format!("{target} does not exist")),
    }
}

/// `source_env_if_exists <file>` — the same, silent when it is not there.
pub fn source_env_if_exists(env: &mut Environment, args: &[String]) -> Result<i32> {
    let Some(target) = args.get(1) else {
        return fault("source_env_if_exists", "needs a path");
    };
    match resolve(target) {
        Some(path) => run(env, &path),
        None => Ok(0),
    }
}

/// `source_up [name]` — the same file, from a directory **above** this one.
///
/// Strictly above, which is the whole point: a monorepo's leaf `.envrc` starts by pulling in the
/// root's, and searching from here would find itself and recurse until the stack ran out.
pub fn source_up(env: &mut Environment, args: &[String]) -> Result<i32> {
    match above(args.get(1)) {
        Some(path) => run(env, &path),
        None => fault("source_up", "no file found above this directory"),
    }
}

/// `source_up_if_exists [name]`
pub fn source_up_if_exists(env: &mut Environment, args: &[String]) -> Result<i32> {
    match above(args.get(1)) {
        Some(path) => run(env, &path),
        None => Ok(0),
    }
}

/// The nearest `name` strictly above the directory being loaded.
fn above(name: Option<&String>) -> Option<PathBuf> {
    let name = name
        .map(String::as_str)
        .unwrap_or(super::super::find::ENVRC);
    upwards(here().parent()?, name)
}

/// A path that may name a directory, in which case its `.envrc` is meant.
fn resolve(target: &str) -> Option<PathBuf> {
    let path = absolute(target, &here());
    if path.is_dir() {
        let inside = path.join(super::super::find::ENVRC);
        return inside.is_file().then_some(inside);
    }
    path.is_file().then_some(path)
}

/// Run `path` as shell, in this environment, and depend on it from now on.
///
/// **Not gated by the allow store, and deliberately.** The file that named it was allowed, and a
/// second prompt for a path chosen by an already-trusted file asks a question the answer to which
/// is already yes. Trust flows from the `.envrc`, which is exactly how direnv treats `source_env`.
fn run(env: &mut Environment, path: &Path) -> Result<i32> {
    watch(path);
    crate::env::builtins::builtin_source(
        env,
        &["source".to_string(), path.to_string_lossy().into_owned()],
    )
}

/// `dotenv [file]` — load a `.env`, the flat `KEY=value` kind.
pub fn dotenv(env: &mut Environment, args: &[String]) -> Result<i32> {
    let path = absolute(args.get(1).map(String::as_str).unwrap_or(".env"), &here());
    if !path.is_file() {
        return fault("dotenv", &format!("{} does not exist", path.display()));
    }
    load(env, &path)
}

/// `dotenv_if_exists [file]`
pub fn dotenv_if_exists(env: &mut Environment, args: &[String]) -> Result<i32> {
    let path = absolute(args.get(1).map(String::as_str).unwrap_or(".env"), &here());
    if !path.is_file() {
        return Ok(0);
    }
    load(env, &path)
}

/// Read a `.env` and export what it holds.
fn load(env: &mut Environment, path: &Path) -> Result<i32> {
    watch(path);
    let Ok(source) = std::fs::read_to_string(path) else {
        return fault("dotenv", &format!("cannot read {}", path.display()));
    };
    for (name, value) in parse(&source) {
        env.set_var(&name, &value, true);
    }
    Ok(0)
}

/// The `.env` grammar, which is small and is not shell.
///
/// A blank line or a `#` comment is skipped, a leading `export ` is allowed because half the files
/// in the wild have one, and the value may be bare, single-quoted or double-quoted. Only the
/// double-quoted form expands escapes — that is the rule everything from Docker to python-dotenv
/// follows, and a `.env` holding a Windows path in single quotes must not have its backslashes
/// eaten.
fn parse(source: &str) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    for line in source.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line).trim_start();
        let Some((name, value)) = line.split_once('=') else {
            continue;
        };
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        pairs.push((name.to_string(), unquote(value.trim())));
    }
    pairs
}

/// The value as written, with quotes taken off and escapes applied only inside double quotes.
fn unquote(value: &str) -> String {
    if value.len() >= 2 && value.starts_with('\'') && value.ends_with('\'') {
        return value[1..value.len() - 1].to_string();
    }
    if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
        let inner = &value[1..value.len() - 1];
        let mut out = String::with_capacity(inner.len());
        let mut chars = inner.chars();
        while let Some(c) = chars.next() {
            match c {
                '\\' => match chars.next() {
                    Some('n') => out.push('\n'),
                    Some('t') => out.push('\t'),
                    Some('r') => out.push('\r'),
                    Some(other) => out.push(other),
                    None => out.push('\\'),
                },
                other => out.push(other),
            }
        }
        return out;
    }
    // Bare: an unquoted trailing comment is a comment, which is what every reader of these does.
    match value.split_once(" #") {
        Some((before, _)) => before.trim_end().to_string(),
        None => value.to_string(),
    }
}

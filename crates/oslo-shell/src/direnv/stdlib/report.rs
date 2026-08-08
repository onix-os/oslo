//! Saying things, asking things, and the small predicates an `.envrc` branches on.

use super::{absolute, fault, here, watch};
use crate::env::Environment;
use oslo_base::error::Result;

/// `log_status <message>` — direnv's own channel, on stderr.
///
/// Stderr and not stdout, because an `.envrc` runs while the shell is between a `cd` and a prompt
/// and its stdout may well be being read by the caller. direnv prefixes every line with its own
/// name so that a project's chatter is attributable; keeping that means a file that says
/// `log_status "using node 20"` reads the same here as it does there.
pub fn log_status(_env: &mut Environment, args: &[String]) -> Result<i32> {
    eprintln!("direnv: {}", args[1..].join(" "));
    Ok(0)
}

/// `log_error <message>`
pub fn log_error(_env: &mut Environment, args: &[String]) -> Result<i32> {
    eprintln!("direnv: {}", args[1..].join(" "));
    Ok(0)
}

/// `has <command>` — whether it could be run.
///
/// Every route counts, in the order the shell would take: a builtin, a function, then `$PATH`. A
/// file that guards `use flake` with `has nix` is asking whether the *shell* can run it, and
/// answering only for `$PATH` would say no to a perfectly good builtin.
pub fn has(env: &mut Environment, args: &[String]) -> Result<i32> {
    let Some(name) = args.get(1) else {
        return Ok(1);
    };
    if env.get_builtin(name).is_some() || env.get_function(name).is_some() {
        return Ok(0);
    }
    Ok(i32::from(!on_path(env, name)))
}

/// Whether `name` is an executable file on `$PATH`.
fn on_path(env: &Environment, name: &str) -> bool {
    if name.contains('/') {
        return is_executable(std::path::Path::new(name));
    }
    let path = env.get_var("PATH").unwrap_or_default();
    path.split(':')
        .filter(|dir| !dir.is_empty())
        .any(|dir| is_executable(&std::path::Path::new(dir).join(name)))
}

fn is_executable(path: &std::path::Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

/// `join_args <word>...` — one line, each word quoted so it survives being re-read.
pub fn join_args(_env: &mut Environment, args: &[String]) -> Result<i32> {
    let joined: Vec<String> = args[1..].iter().map(|word| super::quote(word)).collect();
    println!("{}", joined.join(" "));
    Ok(0)
}

/// `env_vars_required <NAME>...` — refuse to go on without them.
///
/// Named and counted rather than failing on the first: a file that needs three secrets should say
/// so once, not three times over three edits.
pub fn env_vars_required(env: &mut Environment, args: &[String]) -> Result<i32> {
    let missing: Vec<&String> = args[1..]
        .iter()
        .filter(|name| env.get_var(name).is_none_or(str::is_empty))
        .collect();
    if missing.is_empty() {
        return Ok(0);
    }
    for name in &missing {
        eprintln!("direnv: env_vars_required: {name} is not set");
    }
    Ok(1)
}

/// `on_git_branch [name]` — whether this is a git tree, on that branch if one is named.
///
/// `.git/HEAD` is read rather than `git` being run. It is one file, the format has been stable for
/// the entire life of the tool, and an `.envrc` that branches on this runs on every `cd` — spawning
/// a process to answer it would be the most expensive line in most files.
pub fn on_git_branch(_env: &mut Environment, args: &[String]) -> Result<i32> {
    let Some(branch) = current_branch() else {
        return Ok(1);
    };
    match args.get(1) {
        Some(wanted) => Ok(i32::from(&branch != wanted)),
        None => Ok(0),
    }
}

/// The checked-out branch, or `None` outside a tree or with a detached head.
fn current_branch() -> Option<String> {
    let git = super::paths::upwards(&here(), ".git")?;
    // A worktree's `.git` is a file pointing at the real directory.
    let dir = if git.is_file() {
        let pointer = std::fs::read_to_string(&git).ok()?;
        let rest = pointer.trim().strip_prefix("gitdir:")?.trim().to_string();
        absolute(&rest, git.parent()?)
    } else {
        git
    };
    let head = std::fs::read_to_string(dir.join("HEAD")).ok()?;
    Some(head.trim().strip_prefix("ref: refs/heads/")?.to_string())
}

/// `strict_env [command...]` — bash's `set -u` around a command, or from here on.
///
/// **Accepted, and a no-op when it is a mode rather than a wrapper.** In direnv this guards against
/// an `.envrc` reading a variable it never set, in a *subprocess* whose death costs nothing. Here
/// the file runs in the shell, and turning an unset variable into a fatal error for the rest of the
/// session is not a trade an `.envrc` gets to make on the shell's behalf. Given a command it still
/// does the useful half and runs it.
pub fn strict_env(env: &mut Environment, args: &[String]) -> Result<i32> {
    match args.len() {
        0 | 1 => Ok(0),
        _ => super::run(env, &args[1..]),
    }
}

/// `unstrict_env [command...]` — the other half, and the same treatment.
pub fn unstrict_env(env: &mut Environment, args: &[String]) -> Result<i32> {
    strict_env(env, args)
}

/// `direnv_version <wanted>` — the file saying what it was written against.
///
/// Answered rather than refused. oslo is not direnv and any number it reported would be a claim
/// about a different program; what the line is *for* is stopping a file that needs a newer stdlib
/// from failing obscurely, and a missing function here already fails by name.
pub fn direnv_version(_env: &mut Environment, _args: &[String]) -> Result<i32> {
    Ok(0)
}

/// `watch_file <path>...` — reload when one of these changes, not only the `.envrc`.
pub fn watch_file(_env: &mut Environment, args: &[String]) -> Result<i32> {
    if args.len() < 2 {
        return fault("watch_file", "needs a path");
    }
    let base = here();
    for path in &args[1..] {
        watch(&absolute(path, &base));
    }
    Ok(0)
}

/// `watch_dir <dir>` — the same, for everything under a directory.
pub fn watch_dir(_env: &mut Environment, args: &[String]) -> Result<i32> {
    let Some(dir) = args.get(1) else {
        return fault("watch_dir", "needs a directory");
    };
    watch(&absolute(dir, &here()));
    Ok(0)
}

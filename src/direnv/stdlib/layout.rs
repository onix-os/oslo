//! `layout <language>` — the per-language conventions.
//!
//! Each of these is small and each is somebody's whole workflow: `layout python` is how a very
//! large number of `.envrc` files in the world put a virtualenv on `$PATH`. They share one shape —
//! decide a directory under `direnv_layout_dir`, put its `bin` in front, export the variable the
//! toolchain looks for — so they are written once here and differ only in the details that matter.

use super::paths::prepend_into;
use super::{fault, here};
use crate::env::Environment;
use crate::error::Result;
use std::path::PathBuf;

/// `layout <language> [args...]` — dispatch to `layout_<language>`.
///
/// A shell function wins, as with `use`, so a `direnvrc` can add `layout_elixir` or replace one of
/// these outright.
pub fn dispatch(env: &mut Environment, args: &[String]) -> Result<i32> {
    let Some(kind) = args.get(1) else {
        return fault("layout", "needs a language");
    };
    let target = format!("layout_{kind}");
    if env.get_function(&target).is_some() {
        let mut forwarded = vec![target];
        forwarded.extend_from_slice(&args[2..]);
        return super::run(env, &forwarded);
    }
    let rest = &args[2..];
    match kind.as_str() {
        "python" | "python3" | "pyenv" => python(env, rest),
        "poetry" => poetry(env),
        "uv" => uv(env),
        "node" => node(env),
        "go" => go(env),
        "ruby" => ruby(env),
        "php" => simple(env, "php", "vendor/bin"),
        "perl" => perl(env),
        "julia" => julia(env),
        _ => fault("layout", &format!("{kind} is not a layout oslo knows")),
    }
}

/// Where a layout keeps what it builds: `$direnv_layout_dir`, or `.direnv` beside the file.
fn layout_dir(env: &Environment) -> PathBuf {
    env.get_var("direnv_layout_dir")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| here().join(".direnv"))
}

/// `layout python [interpreter]` — a virtualenv, made if it is not there.
///
/// `$VIRTUAL_ENV` and `bin` on the front is the whole contract, and it is what `pip`, `pytest` and
/// every editor's interpreter detection read. The environment is *created* on first arrival because
/// that is what direnv does and because a layout that only worked once somebody had remembered to
/// run `python -m venv` would be a layout nobody could rely on.
fn python(env: &mut Environment, args: &[String]) -> Result<i32> {
    let interpreter = args.first().map(String::as_str).unwrap_or("python3");
    let venv = layout_dir(env).join("python");
    if !venv.join("bin").is_dir() {
        let _ = std::fs::create_dir_all(&venv);
        let made = crate::exec::eval_command_substitution(
            env,
            &format!(
                "{interpreter} -m venv {} 2>&1",
                shell_quote(&venv.to_string_lossy())
            ),
        );
        if made.is_err() || !venv.join("bin").is_dir() {
            return fault(
                "layout python",
                &format!("could not create a virtualenv with {interpreter}"),
            );
        }
    }
    env.set_var("VIRTUAL_ENV", &venv.to_string_lossy(), true);
    // Nothing else is going to say which one is active, and every prompt that shows a virtualenv
    // reads this rather than working it out from `$PATH`.
    env.set_var("VIRTUAL_ENV_PROMPT", "", true);
    prepend_into(
        env,
        "PATH",
        &[venv.join("bin").to_string_lossy().into_owned()],
    )
}

/// `layout poetry` — poetry's own virtualenv, wherever it decided to put it.
fn poetry(env: &mut Environment) -> Result<i32> {
    let path = crate::exec::eval_command_substitution(env, "poetry env info --path 2>/dev/null")
        .unwrap_or_default();
    let path = path.trim().to_string();
    if path.is_empty() {
        return fault(
            "layout poetry",
            "poetry has no environment for this project",
        );
    }
    env.set_var("VIRTUAL_ENV", &path, true);
    prepend_into(env, "PATH", &[format!("{path}/bin")])
}

/// `layout uv` — uv's `.venv`, which it puts in the project by convention.
fn uv(env: &mut Environment) -> Result<i32> {
    let venv = here().join(".venv");
    if !venv.join("bin").is_dir() {
        return fault("layout uv", "no .venv here; run `uv sync` first");
    }
    env.set_var("VIRTUAL_ENV", &venv.to_string_lossy(), true);
    prepend_into(
        env,
        "PATH",
        &[venv.join("bin").to_string_lossy().into_owned()],
    )
}

/// `layout node` — the project's own `node_modules/.bin` first.
fn node(env: &mut Environment) -> Result<i32> {
    simple(env, "node", "node_modules/.bin")
}

/// `layout go` — a `$GOPATH` inside the project rather than one shared by everything you own.
fn go(env: &mut Environment) -> Result<i32> {
    let gopath = layout_dir(env).join("go");
    env.set_var("GOPATH", &gopath.to_string_lossy(), true);
    prepend_into(
        env,
        "PATH",
        &[gopath.join("bin").to_string_lossy().into_owned()],
    )
}

/// `layout ruby` — bundler's per-project gems.
fn ruby(env: &mut Environment) -> Result<i32> {
    let gems = layout_dir(env).join("ruby");
    env.set_var("GEM_HOME", &gems.to_string_lossy(), true);
    env.set_var("BUNDLE_BIN", &gems.join("bin").to_string_lossy(), true);
    prepend_into(
        env,
        "PATH",
        &[gems.join("bin").to_string_lossy().into_owned()],
    )
}

/// `layout perl` — a local::lib under the layout directory.
fn perl(env: &mut Environment) -> Result<i32> {
    let root = layout_dir(env).join("perl5");
    env.set_var("PERL_LOCAL_LIB_ROOT", &root.to_string_lossy(), true);
    env.set_var(
        "PERL_MB_OPT",
        &format!("--install_base \"{}\"", root.display()),
        true,
    );
    env.set_var(
        "PERL_MM_OPT",
        &format!("INSTALL_BASE={}", root.display()),
        true,
    );
    prepend_into(
        env,
        "PERL5LIB",
        &[root.join("lib/perl5").to_string_lossy().into_owned()],
    )?;
    prepend_into(
        env,
        "PATH",
        &[root.join("bin").to_string_lossy().into_owned()],
    )
}

/// `layout julia` — a project-local depot.
fn julia(env: &mut Environment) -> Result<i32> {
    let depot = layout_dir(env).join("julia");
    env.set_var("JULIA_DEPOT_PATH", &depot.to_string_lossy(), true);
    prepend_into(
        env,
        "PATH",
        &[depot.join("bin").to_string_lossy().into_owned()],
    )
}

/// The common shape: one relative directory of the project on the front of `$PATH`.
fn simple(env: &mut Environment, _name: &str, relative: &str) -> Result<i32> {
    prepend_into(
        env,
        "PATH",
        &[here().join(relative).to_string_lossy().into_owned()],
    )
}

fn shell_quote(word: &str) -> String {
    format!("'{}'", word.replace('\'', "'\\''"))
}

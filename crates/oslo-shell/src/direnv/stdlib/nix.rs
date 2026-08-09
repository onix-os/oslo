//! `use`, and the two things it is nearly always used for.
//!
//! `use <thing>` is direnv's one extension point: it calls `use_<thing>`, so a `direnvrc` can add
//! `use_java` and every project gets it. That indirection is kept exactly, because it is what makes
//! a personal `direnvrc` worth having — see [`super::super::rcfile`].

use super::super::devshell;
use super::{fault, here, watch};
use crate::env::Environment;
use oslo_base::error::Result;

/// `use <thing> [args...]` — dispatch to `use_<thing>`.
///
/// A shell function wins over the builtin, which is how a `direnvrc` overrides or adds one. Looked
/// up by name at call time rather than resolved when the stdlib is installed, so a `direnvrc` that
/// defines `use_java` *after* something else sourced it still works.
pub fn use_dispatch(env: &mut Environment, args: &[String]) -> Result<i32> {
    let Some(thing) = args.get(1) else {
        return fault("use", "needs something to use");
    };
    let target = format!("use_{thing}");
    let mut forwarded = vec![target.clone()];
    forwarded.extend_from_slice(&args[2..]);

    if env.get_function(&target).is_some() {
        return super::run(env, &forwarded);
    }
    match env.get_builtin(&target) {
        Some(func) => func(env, &forwarded),
        None => fault(
            "use",
            &format!("{thing} is not something oslo knows how to use"),
        ),
    }
}

/// `use flake [installable]` — the environment of a flake's dev shell.
pub fn use_flake(env: &mut Environment, args: &[String]) -> Result<i32> {
    // Everything after `use flake` belongs to `nix print-dev-env`, installable and flags alike —
    // `use flake --option warn-dirty false` is a real line in a real `.envrc`. Reading the first
    // one as the installable passed `--option` to nix as a flake to build.
    let forwarded = &args[1.min(args.len())..];
    // Both, and before the work: a `flake.lock` that moves is a different dev shell, and a file
    // that fails to build should still reload when you fix the flake that broke it.
    let base = here();
    watch(&base.join("flake.nix"));
    watch(&base.join("flake.lock"));
    match devshell::apply(env, forwarded) {
        Ok(count) => {
            eprintln!("direnv: use flake: {count} variables");
            Ok(0)
        }
        Err(problem) => fault("use flake", &problem),
    }
}

/// `use nix [args...]` — the same, for a `shell.nix` or `default.nix`.
///
/// `print-dev-env` reads both kinds of expression, so the difference between this and `use flake`
/// is only which installable is named. direnv's `use_nix` predates flakes and does considerably
/// more work with `nix-shell --pure` and a cached dump; this is the modern equivalent, and a file
/// that says `use nix` gets a working dev shell rather than a refusal.
pub fn use_nix(env: &mut Environment, args: &[String]) -> Result<i32> {
    let base = here();
    for name in ["shell.nix", "default.nix"] {
        watch(&base.join(name));
    }
    let forwarded = &args[1.min(args.len())..];
    match devshell::apply(env, forwarded) {
        Ok(count) => {
            eprintln!("direnv: use nix: {count} variables");
            Ok(0)
        }
        Err(problem) => fault("use nix", &problem),
    }
}

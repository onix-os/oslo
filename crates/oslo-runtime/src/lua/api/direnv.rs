//! `oslo.direnv` — the library a `.env.lua` is written against.
//!
//! A namespace rather than more functions on `oslo`, because these are for one job. `oslo` itself
//! had grown twenty-four flat entries with `nix_develop` sitting between `login` and `path_add`,
//! which tells a reader nothing about which of them belong together. `fs`, `json`, `path`, `proc`
//! and `re` were already tables; this is the same idea applied to the directory environment.
//!
//! # Thin, on purpose
//!
//! Every function here is a call into [`oslo_shell::direnv`], which is also what an `.envrc` reaches
//! through [`oslo_shell::direnv::stdlib`]. That matters more than it looks: `nix_develop` and `use flake`
//! do the same delicate thing — take a dev shell's environment from `nix print-dev-env --json`
//! while withholding the handful of variables that would wreck the shell you are standing in — and
//! two copies of that list is two chances to get it wrong, silently, in a way nobody would think to
//! test. See [`oslo_shell::direnv::devshell`] for what those are and why.

use super::util::{put, text};
use oslo_base::value::{LuaError, Table, Value};
use oslo_shell::direnv::stdlib;
use oslo_shell::env::Environment;
#[cfg(feature = "nix")]
use oslo_shell::nix_shell as devshell;
use std::sync::{Arc, Mutex};

/// Build the `oslo.direnv` table.
pub fn build(env: &Arc<Mutex<Environment>>) -> Value {
    let mut it = Table::new();
    // `oslo.direnv.nix_develop` is the seam: a directory file asking for a dev shell. It needs
    // both halves, and a build with only one of them simply does not offer the name.
    #[cfg(feature = "nix")]
    nix_develop(&mut it, env);
    path_add(&mut it, env);
    Value::table(it)
}

fn path_add(it: &mut Table, env: &Arc<Mutex<Environment>>) {
    // oslo.direnv.path_add(dir, [var]) — put `dir` on the front of `$PATH`, or of the named variable.
    //
    // The single most common thing a `.env.lua` does, and spelling it by hand is both wordy and
    // easy to get subtly wrong: forgetting the separator, or appending rather than prepending so
    // the project's own tool loses to the system one. Relative paths resolve against the current
    // directory, because a directory environment saying `./bin` means *its* bin.
    //
    // Idempotent, so a reload does not grow the variable each time. This is `PATH_add` from an
    // `.envrc` under another name — one implementation, so the two cannot drift.
    let env = Arc::clone(env);
    put(it, "path_add", move |_, args| {
        let dir = text(&args, 1, "oslo.direnv.path_add")?.to_string();
        let name = match args.get(1) {
            Some(Value::Str(name)) => name.to_string(),
            _ => "PATH".to_string(),
        };
        let mut guard = crate::lua::engine::borrow_env(&env)?;
        stdlib::prepend_into(&mut guard, &name, &[dir])
            .map_err(|e| LuaError::new(format!("oslo.direnv.path_add: {e}")))?;
        let joined = guard.get_var(&name).unwrap_or_default().to_string();
        Ok(vec![Value::str(joined)])
    });
}

#[cfg(feature = "nix")]
fn nix_develop(it: &mut Table, env: &Arc<Mutex<Environment>>) {
    let env = Arc::clone(env);
    put(it, "nix_develop", move |_, args| {
        // `oslo.direnv.nix_develop()` means this directory's flake; a string names another installable,
        // exactly as `use flake ..#other` does. A table asks for options.
        let (forwarded, want) = match args.first() {
            Some(Value::Str(_)) => (
                vec![text(&args, 1, "oslo.direnv.nix_develop")?.to_string()],
                devshell::Want::default(),
            ),
            // `nix_develop{ hook = true, functions = true }`, and optionally the installable.
            Some(Value::Table(options)) => {
                let options = options.borrow();
                let named = match options.get_str("flake") {
                    Value::Str(name) => vec![name.to_string()],
                    _ => Vec::new(),
                };
                let on = |key: &str| matches!(options.get(&Value::str(key)), Value::Bool(true));
                (
                    named,
                    devshell::Want {
                        hook: on("hook"),
                        functions: on("functions"),
                    },
                )
            }
            // Nothing named: `print-dev-env` resolves this directory's flake by itself, which is
            // what a bare `use flake` relies on too.
            _ => (Vec::new(), devshell::Want::default()),
        };
        let count = {
            let mut guard = crate::lua::engine::borrow_env(&env)?;
            devshell::apply_with(&mut guard, &forwarded, want)
                .map_err(|e| LuaError::new(format!("oslo.direnv.nix_develop: {e}")))?
        };
        Ok(vec![Value::Number(oslo_base::value::Number::Int(
            count as i64,
        ))])
    });
}

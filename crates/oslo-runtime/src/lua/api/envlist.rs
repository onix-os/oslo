//! `oslo.env.path` and the family around it — a colon-separated variable as the list it is.
//!
//! ```lua
//! oslo.env.path_add("~/.local/bin")             -- on the front, once, absolute
//! oslo.env.path_add("./node_modules/.bin")      -- relative to where you are now
//! oslo.env.path_add("/opt/fallback", { last = true })
//! oslo.env.path_add("/usr/share/man", { var = "MANPATH" })
//!
//! for _, dir in ipairs(oslo.env.path()) do … end
//! oslo.env.path_remove("/nix/*")                -- a pattern; answers how many went
//! if oslo.env.has_path("~/.cargo/bin") then … end
//! ```
//!
//! # Why these are not `oslo.env.set("PATH", …)`
//!
//! Because that is string surgery, and every one of its edge cases is a real one somebody hits:
//! forgetting the separator, appending where prepending was meant so the project's own tool loses
//! to the system one, a reload that grows the variable by an entry every time, `./bin` resolving
//! against wherever the shell later stands rather than where it was written, and the empty entry a
//! trailing colon leaves behind — which means "the current directory" to the dynamic linker, and is
//! a way to run something you did not mean to run. See [`oslo_shell::env::lists`].
//!
//! # In every build
//!
//! These were `oslo.direnv.path_add`, behind the `direnv` feature. Putting a directory on `$PATH`
//! is not a directory-environment thing that configurations also happen to do; it is the single
//! most common thing any configuration does. One implementation serves both, so `PATH_add` in an
//! `.envrc` and `oslo.env.path_add` in an `init.lua` cannot drift apart.

use super::util::{list, ok, opt_text, put, text};
use crate::lua::engine::borrow_env;
use oslo_base::value::{Table, Value};
use oslo_shell::env::Environment;
use oslo_shell::env::lists;
use std::sync::{Arc, Mutex};

/// Add the list functions to the `oslo.env` table.
pub fn install(env_table: &mut Table, env: &Arc<Mutex<Environment>>) {
    // oslo.env.path([var]) -> { "/usr/bin", "/bin" }
    let it = Arc::clone(env);
    put(env_table, "path", move |_, args| {
        let name = opt_text(&args, 1, "oslo.env.path")?.unwrap_or_else(|| "PATH".into());
        let guard = borrow_env(&it)?;
        ok(list(
            lists::entries(&guard, &name).into_iter().map(Value::str),
        ))
    });

    // oslo.env.path_add(dir, { last = false, var = "PATH" }) -> the variable's new value
    //
    // The options are a table rather than positional, because `path_add(dir, true)` at a call site
    // says nothing about what the `true` decides. Idempotent either way: an entry already present
    // is moved to the front when prepending, and left where it is when appending — moving it would
    // quietly demote something the caller had deliberately preferred.
    let it = Arc::clone(env);
    put(env_table, "path_add", move |_, args| {
        // `~/.local/bin` is what people write, and nothing expands it on the way in from Lua —
        // an `.envrc` gets the shell's own expansion first, a config does not. Without this the
        // entry is a literal `~` directory that will never match anything.
        let dir = super::path::expand_tilde(&text(&args, 1, "oslo.env.path_add")?);
        let (name, last) = options(args.get(1));
        let mut guard = borrow_env(&it)?;
        let base = lists::here();
        if last {
            lists::append(&mut guard, &name, &[dir], &base);
        } else {
            lists::prepend(&mut guard, &name, &[dir], &base);
        }
        ok(Value::str(guard.get_var(&name).unwrap_or_default()))
    });

    // oslo.env.path_remove(pattern, { var = "PATH" }) -> how many entries went
    //
    // A pattern, not a path, because the reason to remove one is usually "everything under there"
    // — `oslo.env.path_remove("/nix/*")` after leaving a dev shell. `*` crosses `/`, and an entry
    // named here need not exist on disk.
    let it = Arc::clone(env);
    put(env_table, "path_remove", move |_, args| {
        let pattern = text(&args, 1, "oslo.env.path_remove")?;
        let (name, _) = options(args.get(1));
        let mut guard = borrow_env(&it)?;
        ok(Value::int(
            lists::remove(&mut guard, &name, &[pattern]) as i64
        ))
    });

    // oslo.env.has_path(dir, { var = "PATH" }) -> whether it is already there
    //
    // Compared the way `path_add` would have written it, so `has_path("./bin")` answers about the
    // absolute entry rather than about the two characters `./`.
    let it = Arc::clone(env);
    put(env_table, "has_path", move |_, args| {
        let dir = super::path::expand_tilde(&text(&args, 1, "oslo.env.has_path")?);
        let (name, _) = options(args.get(1));
        let guard = borrow_env(&it)?;
        ok(Value::Bool(lists::contains(
            &guard,
            &name,
            &dir,
            &lists::here(),
        )))
    });
}

/// The optional second argument: which variable, and which end.
///
/// Anything that is not a table is ignored rather than refused — the common call has no second
/// argument at all, and a stray one is not worth stopping a configuration for.
fn options(value: Option<&Value>) -> (String, bool) {
    let Some(Value::Table(table)) = value else {
        return ("PATH".to_string(), false);
    };
    let table = table.borrow();
    let name = match table.get_str("var") {
        Value::Str(name) => name.to_string(),
        _ => "PATH".to_string(),
    };
    (name, table.get_str("last").truthy())
}

#[cfg(test)]
#[path = "envlist/tests.rs"]
mod tests;

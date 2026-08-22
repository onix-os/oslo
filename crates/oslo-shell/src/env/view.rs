//! The shell, as one Lua record, for a caller that already holds the state.
//!
//! # Why this exists at all
//!
//! A Lua builtin runs *while the shell holds its own state*, so every `oslo.*` call that reaches
//! back for it raises. That is a small list — 28 functions — but it contains the ones a builtin most
//! wants: `$?`, a variable's value, the positional parameters. A builtin could not see the status of
//! the command before it **by any route**: the `oslo.*` call raises, and the Lua-global spelling
//! `try_lock`s and answers nil.
//!
//! The fix is not to release the lock. It is to stop asking. The frame above already holds a live
//! `&mut Environment` — `env::scope::registry` clones the callback out of the registry
//! precisely so it can — and building a record from it costs no new borrow at all.
//!
//! This lives beside `Environment` rather than in the Lua layer for one reason: two of the four
//! answering hooks fire from inside this crate, and this crate cannot see `oslo-runtime`.
//!
//! # A narrowed view, not the store
//!
//! `oslo_ui::shell::Shell` is the same idea for the interface layer — nine methods rather than the
//! `Environment` itself. This is that argument applied to Lua: a record of facts, not a handle.
//!
//! # Cheap by default, and expensive only when asked
//!
//! Everything in the default record is a field read or a small clone. The collections are not:
//! `all_vars` clones every name and value *and sorts them*, and a builtin used inside a `for` loop
//! would pay that on every iteration for data it never reads. So they are behind `Wants`, parsed
//! once at declaration into a bitset.
//!
//! **A key that was not asked for raises rather than answering nil**, which is what makes `wants`
//! safe to have. `shell.aliases` when the declaration omitted `"aliases"` says so and names the
//! field to add; without that, `shell.aliases and shell.aliases[x]` would be a silent nothing —
//! the exact shape of bug this whole module is fixing.

use crate::env::Environment;
use oslo_base::value::{LuaError, Table, Value};
use std::rc::Rc;

/// Which of the expensive collections a caller asked for.
///
/// `Copy`, so it rides in the `'static` closure a dynamic builtin is registered as.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct Wants(u8);

impl Wants {
    /// Every name `wants` accepts, in the order the error message lists them.
    pub const NAMES: &'static [&'static str] = &[
        "vars",
        "aliases",
        "functions",
        "builtins",
        "dirstack",
        "stack",
    ];

    /// The bit for `name`, or `None` when it is not one of [`Self::NAMES`].
    ///
    /// The caller raises on `None` — at *declaration* time, so `wants = {"variables"}` is a config
    /// error naming the accepted set rather than a field that is mysteriously missing at call time.
    pub fn bit(name: &str) -> Option<Wants> {
        let at = Self::NAMES.iter().position(|known| *known == name)?;
        Some(Wants(1 << at))
    }

    pub fn with(self, other: Wants) -> Wants {
        Wants(self.0 | other.0)
    }

    pub fn has(self, other: Wants) -> bool {
        self.0 & other.0 != 0
    }

    fn wants(self, name: &str) -> bool {
        Self::bit(name).is_some_and(|bit| self.has(bit))
    }
}

/// The shell as a Lua table, for `owner` — a builtin's name or a hook's, used in the refusal.
pub fn record(env: &Environment, owner: &str, wants: Wants) -> Value {
    let mut it = Table::new();

    it.set_str("name", Value::str(owner));
    // `$?`. The headline: unreachable from a builtin by every other route.
    it.set_str("status", Value::int(env.last_status as i64));
    // The predicate beside the number, because that is what almost every caller actually tests.
    // `Context::to_lua` carries `ok` for the same reason.
    it.set_str("ok", Value::Bool(env.last_status == 0));
    it.set_str("pid", Value::int(env.pid as i64));
    if let Some(bg) = env.last_bg_pid {
        it.set_str("last_bg_pid", Value::int(bg as i64));
    }
    if let Some(cwd) = env.get_var("PWD") {
        it.set_str("cwd", Value::str(cwd));
    }
    if let Some(old) = env.get_var("OLDPWD") {
        it.set_str("oldcwd", Value::str(old));
    }
    it.set_str("interactive", Value::Bool(env.interactive()));
    // `$-`.
    it.set_str("flags", Value::str(env.option_flags()));
    it.set_str("depth", Value::int(env.call_stack().len() as i64));

    // **1-based, so `$1` is `positional[1]`.** The asymmetry with `argv` — where `argv[1]` is the
    // builtin's own name — is deliberate: that contract is already documented and is not worth
    // breaking for symmetry with a new one.
    it.set_str(
        "positional",
        sequence(env.get_positional().iter().map(Value::str)),
    );
    it.set_str("argc", Value::int(env.get_positional().len() as i64));
    it.set_str(
        "pipestatus",
        sequence(env.pipeline_status().iter().map(|s| Value::int(*s as i64))),
    );

    if wants.wants("vars") {
        // One walk for both: `all_vars` already carries the export flag as its third field, and
        // asking twice would sort twice.
        let mut vars = Table::new();
        let mut exported = Table::new();
        for (name, value, is_exported) in env.all_vars() {
            if is_exported {
                exported.set(Value::str(&name), Value::Bool(true));
            }
            vars.set(Value::str(name), Value::str(value));
        }
        it.set_str("vars", Value::table(vars));
        it.set_str("exported", Value::table(exported));
    }
    if wants.wants("aliases") {
        let mut aliases = Table::new();
        for (name, body) in env.get_aliases() {
            aliases.set(Value::str(name), Value::str(body));
        }
        it.set_str("aliases", Value::table(aliases));
    }
    if wants.wants("functions") {
        it.set_str(
            "functions",
            sequence(env.get_functions().keys().map(Value::str)),
        );
    }
    if wants.wants("builtins") {
        it.set_str("builtins", sequence(env.builtin_names().map(Value::str)));
    }
    if wants.wants("dirstack") {
        it.set_str(
            "dirstack",
            sequence(
                env.get_dir_stack()
                    .iter()
                    .map(|p| Value::str(p.display().to_string())),
            ),
        );
    }
    if wants.wants("stack") {
        it.set_str("stack", sequence(env.call_stack().iter().map(Value::str)));
    }

    let mut table = Table::new();
    std::mem::swap(&mut table, &mut it);
    with_refusal(table, owner, wants)
}

/// A 1-based Lua list.
fn sequence(values: impl Iterator<Item = Value>) -> Value {
    let mut list = Table::new();
    for (i, value) in values.enumerate() {
        list.set(Value::int(i as i64 + 1), value);
    }
    Value::table(list)
}

/// Attach the metatable that refuses a known-but-ungathered key.
///
/// **This is what makes `wants` worth having rather than a trap.** Without it, forgetting an entry
/// gives `nil`, and `shell.vars and shell.vars.HOME` quietly answers nothing — indistinguishable
/// from the variable being unset. With it, the mistake names itself and says how to fix it.
///
/// An unknown key falls through to `nil`, which is ordinary Lua and must stay that way: a caller
/// testing `shell.something_new` to find out whether this build has it is doing a reasonable thing.
fn with_refusal(table: Table, owner: &str, wants: Wants) -> Value {
    let missing: Vec<String> = Wants::NAMES
        .iter()
        .filter(|name| !wants.wants(name))
        .map(|name| (*name).to_string())
        .collect();
    if missing.is_empty() {
        return Value::table(table);
    }
    let owner = owner.to_string();
    let mut meta = Table::new();
    meta.set_str(
        "__index",
        Value::Function(Rc::new(oslo_base::value::Function::Held(Rc::new(
            oslo_luavm::Native {
                name: "shell.__index",
                call: Box::new(move |_, args| {
                    let Some(Value::Str(key)) = args.get(1) else {
                        return Ok(vec![Value::Nil]);
                    };
                    if missing.iter().any(|name| name == key.as_ref()) {
                        return Err(LuaError::new(format!(
                            "{owner}: shell.{key} was not gathered; \
                             add wants = {{ \"{key}\" }} to the declaration"
                        )));
                    }
                    Ok(vec![Value::Nil])
                }),
            },
        )))),
    );
    let it = Rc::new(std::cell::RefCell::new(table));
    it.borrow_mut().metatable = Some(Rc::new(std::cell::RefCell::new(meta)));
    Value::Table(it)
}

#[cfg(test)]
#[path = "view/tests.rs"]
mod tests;

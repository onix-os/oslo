//! `oslo.command` — which programs on `$PATH` this directory can see.
//!
//! ```lua
//! oslo.command.when("direnv", function(dir)                 -- re-decided on every move
//!   return oslo.fs.find_up(".envrc", dir) ~= nil
//! end)
//!
//! oslo.command.hidden()                                     -- { "direnv" }
//! ```
//!
//! See [`oslo_base::command`] for what hiding a name does and why `$PATH` itself is left alone.
//! This module is only the Lua surface and the predicate registry.
//!
//! # The sibling of `oslo.feature.when`, deliberately
//!
//! Same shape, same moment, same mask-not-assignment property — [`super::feature`] gates a builtin
//! and this gates a program, and a configuration usually wants both:
//!
//! ```lua
//! oslo.feature.when("direnv", function(d) return oslo.fs.find_up(".env.lua", d) ~= nil end)
//! oslo.command.when("direnv", function(d) return oslo.fs.find_up(".envrc",   d) ~= nil end)
//! ```
//!
//! Two registrars rather than one because the sets are different in kind: features come from a
//! fixed table, where a name nothing gates is a typo worth refusing, and programs are open-ended,
//! where the whole point is naming one the shell has never heard of. There is nothing to check a
//! command name against, so nothing is checked.
//!
//! # True means visible
//!
//! The predicate answers "can this directory see it", not "hide it". That reads the way the config
//! above reads — *direnv is available where there is an `.envrc`* — and it matches
//! `oslo.feature.when`, where true also means the thing works.
//!
//! Returning nothing is **not** an answer and leaves the name as it was, for the same reason it
//! does for a feature: a handler that forgot a `return` would otherwise hide a command.

use super::util::{list, ok, put, text};
use crate::lua::engine::Registry;
use oslo_base::value::{LuaError, Table, Value};
use std::collections::BTreeSet;
use std::rc::Rc;

/// Registry key prefix for the predicate attached to a command name.
const PREDICATE_KEY: &str = "command:";

/// Registry key holding every name `when` has been called for.
///
/// In the registry rather than in a `static` here so that **reloading the configuration forgets
/// them**: a reload builds a fresh `oslo` table and a fresh registry, and a name whose predicate is
/// gone must stop being asked about — otherwise a command removed from the config would stay
/// hidden until the shell was restarted, with nothing left to explain why.
const NAMES_KEY: &str = "command:__names";

/// Build the `oslo.command` table.
pub(crate) fn build(registry: &Registry) -> Value {
    let mut table = Table::new();

    let store = Rc::clone(registry);
    put(&mut table, "when", move |_, args| {
        let name = text(&args, 1, "oslo.command.when")?;
        // A word with a slash is a path and never goes through a `$PATH` search, so a predicate on
        // one could never be consulted. Refused rather than accepted and ignored.
        if name.contains('/') {
            return Err(LuaError::new(format!(
                "oslo.command.when: {name:?} is a path, not a command name — only a bare word is \
                 looked up on $PATH"
            )));
        }
        let Some(handler @ Value::Function(_)) = args.get(1) else {
            return Err(LuaError::new(format!(
                "oslo.command.when: {name} needs a function, got {}",
                args.get(1).map_or("no value", Value::type_name)
            )));
        };

        let mut held = store.borrow_mut();
        held.insert(format!("{PREDICATE_KEY}{name}"), handler.clone());

        // Registering the same name twice replaces the predicate and does not list it twice, which
        // is what makes a config safe to re-read.
        let mut names: Vec<Value> = match held.get(NAMES_KEY) {
            Some(Value::Table(known)) => known.borrow().sequence().to_vec(),
            _ => Vec::new(),
        };
        if !names.iter().any(|known| known.to_display() == name) {
            names.push(Value::str(&name));
            held.insert(NAMES_KEY.to_string(), list(names));
        }
        ok(Value::Bool(true))
    });

    put(&mut table, "hidden", |_, _| {
        Ok(vec![list(
            oslo_base::command::listing().iter().map(Value::str),
        )])
    });

    Value::table(table)
}

/// Re-decide every command that has a predicate, for the directory the shell is now in.
///
/// **The whole set at once**, because [`oslo_base::command::apply`] replaces rather than adds:
/// that is what makes leaving a directory the same operation as arriving in one.
///
/// Costs nothing for a configuration that never called `when` — one registry lookup that misses.
pub fn decide(dir: &std::path::Path) {
    let Some(Value::Table(names)) = crate::lua::engine::host_value(NAMES_KEY) else {
        return;
    };
    let names: Vec<String> = names
        .borrow()
        .sequence()
        .iter()
        .map(Value::to_display)
        .collect();

    let shown = dir.display().to_string();
    let mut hide = BTreeSet::new();
    for name in names {
        let Some(predicate) = crate::lua::engine::host_value(&format!("{PREDICATE_KEY}{name}"))
        else {
            continue;
        };
        let visible = match crate::lua::engine::call_here(&predicate, vec![Value::str(&shown)]) {
            // Nothing returned is not an answer: the name stays as this directory found it.
            Ok(values) => match values.first() {
                None | Some(Value::Nil) => !oslo_base::command::hidden(&name),
                Some(answer) => answer.truthy(),
            },
            // Reported and skipped, and the name is left **visible**: a predicate that raises must
            // not be able to make a command disappear, which is the failure nobody would diagnose.
            Err(e) => {
                eprintln!("oslo: command {name}: {e}");
                true
            }
        };
        if !visible {
            hide.insert(name);
        }
    }

    // The command index is built from `$PATH` with the hidden names taken out, so it is stale
    // exactly when the set moved — and rebuilding it walks `$PATH`, which is not something to do on
    // every prompt for an answer that did not change.
    if oslo_base::command::apply(hide) {
        oslo_ui::invalidate_command_cache();
    }
}

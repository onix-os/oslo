//! `oslo.feature` — turning parts of the shell off and on while it runs.
//!
//! ```lua
//! oslo.feature.set("direnv", false)
//! oslo.feature.get("direnv")                                -- false
//! for _, f in ipairs(oslo.feature.list()) do … end          -- { name, on, about }
//!
//! oslo.feature.when("direnv", function(dir)                 -- re-decided on every move
//!   return not envrc_above(dir)
//! end)
//! ```
//!
//! See [`crate::feature`] for what a feature is and why the state is a mask rather than an
//! assignment. This module is only the Lua surface and the predicate registry.
//!
//! # `set` and `when` are alternatives, not layers
//!
//! `when` **replaces** any `set` for the same feature: a predicate is re-evaluated on every
//! directory change, so leaving an imperative value underneath it would mean the `set` appeared to
//! work and was then silently overwritten by the next `cd`. Registering a predicate therefore takes
//! ownership of that feature, and `set` on a feature that has one is an error rather than a write
//! nothing will honour.

use super::util::{list, ok, put, record, text};
use crate::feature;
use crate::lua::engine::Registry;
use crate::lua::eval::LuaError;
use crate::lua::eval::value::{Table, Value};
use std::rc::Rc;

/// Registry key prefix for the predicate attached to a feature.
///
/// In the host registry beside the hook handlers and the prompt function, for the same reason they
/// are: a Lua global could be overwritten by a script that chose an unlucky name, and `pairs(_G)`
/// should not walk the shell's internals.
const PREDICATE_KEY: &str = "feature:";

/// Build the `oslo.feature` table.
pub(crate) fn build(registry: &Registry) -> Value {
    let mut table = Table::new();

    put(&mut table, "set", |_, args| {
        let name = text(&args, 1, "oslo.feature.set")?;
        let index = resolve(&name, "set")?;
        if has_predicate_for(&name) {
            return Err(LuaError::new(format!(
                "oslo.feature.set: {name} is decided by oslo.feature.when, so setting it here \
                 would be undone on the next directory change; change the predicate instead"
            )));
        }
        // Anything but `false` and `nil` is on, which is Lua's own truthiness and what a caller
        // writing `oslo.feature.set("vi", want_vi)` means.
        let enabled = args.get(1).map(Value::truthy).unwrap_or(true);
        feature::set(index, enabled);
        ok(Value::Bool(enabled))
    });

    put(&mut table, "get", |_, args| {
        let name = text(&args, 1, "oslo.feature.get")?;
        ok(Value::Bool(feature::on(resolve(&name, "get")?)))
    });

    put(&mut table, "list", |_, _| {
        ok(list(feature::listing().into_iter().map(
            |(name, on, about)| {
                record(vec![
                    ("name", Value::str(name)),
                    ("on", Value::Bool(on)),
                    ("about", Value::str(about)),
                ])
            },
        )))
    });

    let store = Rc::clone(registry);
    put(&mut table, "when", move |_, args| {
        let name = text(&args, 1, "oslo.feature.when")?;
        resolve(&name, "when")?;
        let Some(handler @ Value::Function(_)) = args.get(1) else {
            return Err(LuaError::new(format!(
                "oslo.feature.when: {name} needs a function, got {}",
                args.get(1).map_or("no value", Value::type_name)
            )));
        };
        store
            .borrow_mut()
            .insert(format!("{PREDICATE_KEY}{name}"), handler.clone());
        ok(Value::Bool(true))
    });

    Value::table(table)
}

/// The index of `name`, or a Lua error naming the function that was called.
///
/// **A refusal, never a shrug.** `oslo.feature.set("direnvv", false)` that is quietly obeyed leaves
/// a config which looks like it works and does nothing, which is the failure the fixed table in
/// [`crate::feature`] exists to prevent.
fn resolve(name: &str, function: &str) -> Result<usize, LuaError> {
    feature::resolve(name).ok_or_else(|| {
        let known: Vec<&str> = feature::FEATURES.iter().map(|f| f.name).collect();
        LuaError::new(format!(
            "oslo.feature.{function}: no feature called {name:?}. There is: {}",
            known.join(", ")
        ))
    })
}

/// Whether a predicate is attached to `name`, asked against the interpreter on this thread.
fn has_predicate_for(name: &str) -> bool {
    crate::lua::engine::host_value(&format!("{PREDICATE_KEY}{name}")).is_some()
}

/// Re-decide every feature that has a predicate, for the directory the shell is now in.
///
/// **Recomputed rather than remembered**, which is the whole point of the predicate form: there is
/// no "restore on the way out" to get wrong, because leaving a directory simply asks the question
/// again and gets the other answer.
///
/// Cheap when unused — one registry lookup per feature that has a predicate, and none at all for a
/// config that never called `when`.
pub fn decide(dir: &std::path::Path) {
    let shown = dir.display().to_string();
    for (index, f) in feature::FEATURES.iter().enumerate() {
        let key = format!("{PREDICATE_KEY}{}", f.name);
        let Some(predicate) = crate::lua::engine::host_value(&key) else {
            continue;
        };
        match crate::lua::engine::call_here(&predicate, vec![Value::str(&shown)]) {
            Ok(values) => {
                // Nothing returned is *not* an answer, and reading it as `false` would turn a
                // feature off because a handler forgot a `return`. Left alone instead.
                match values.first() {
                    None | Some(Value::Nil) => {}
                    Some(answer) => feature::set(index, answer.truthy()),
                }
            }
            // Reported and skipped: a predicate that raises must not decide the feature by
            // accident, and must not stop the others being asked.
            Err(e) => eprintln!("oslo: feature {}: {e}", f.name),
        }
    }
}

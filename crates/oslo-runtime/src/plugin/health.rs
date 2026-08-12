//! `oslo.plugin.health` — a check a plugin writes about itself.
//!
//! ```lua
//! oslo.plugin.health("notes", function(report)
//!   if oslo.proc.which("age") then report.ok("age is installed")
//!   else report.bad("age is not installed; the vault cannot be locked") end
//! end)
//! ```
//!
//! **Only the plugin knows what it needs.** The shell can check that a plugin is installed, trusted
//! and not shadowed — `doctor` does — but whether the external tool it shells out to exists, or
//! whether its database is writable, is a question only its own code can ask. neovim's `vim.health`
//! exists for exactly this, and `:checkhealth` is well liked because "it is installed and nothing
//! happens" is the question a plugin system is asked most.
//!
//! Registered when the plugin loads, which is why `doctor` has to load a plugin to ask it.

use super::doctor::State;
use oslo_lua::value::{Table, Value};
use std::cell::RefCell;
use std::collections::HashMap;

thread_local! {
    /// Plugin name to the checks it registered.
    static CHECKS: RefCell<HashMap<String, Vec<Value>>> = RefCell::new(HashMap::new());
}

/// Build the `oslo.plugin` table.
pub fn build() -> Value {
    let mut plugin = Table::new();
    // oslo.plugin.health(name, function(report) … end)
    super::super::lua::api::util::put(&mut plugin, "health", |_, args| {
        let Some(Value::Str(name)) = args.first() else {
            return Err(oslo_lua::LuaError::new(
                "oslo.plugin.health: the first argument is the plugin's name".to_string(),
            ));
        };
        let Some(check @ Value::Function(_)) = args.get(1) else {
            return Err(oslo_lua::LuaError::new(
                "oslo.plugin.health: the second argument must be a function".to_string(),
            ));
        };
        CHECKS.with(|slot| {
            slot.borrow_mut()
                .entry(name.to_string())
                .or_default()
                .push(check.clone())
        });
        Ok(vec![Value::Bool(true)])
    });
    // The other half of the same table: what a plugin says about itself *here* is a health check,
    // and what it says about itself anywhere is a test.
    super::test::install(&mut plugin);
    Value::table(plugin)
}

/// Run every check `name` registered, and answer what they said.
///
/// The `report` handed to a check is three functions rather than a returned value, so a check that
/// wants to say several things does not have to build a list — and so a check that says nothing at
/// all is a check that found nothing wrong.
pub fn run(name: &str) -> Vec<(State, String)> {
    let checks = CHECKS.with(|slot| slot.borrow().get(name).cloned().unwrap_or_default());
    if checks.is_empty() {
        return Vec::new();
    }
    let said: std::rc::Rc<RefCell<Vec<(State, String)>>> =
        std::rc::Rc::new(RefCell::new(Vec::new()));

    let mut report = Table::new();
    for (field, state) in [
        ("ok", State::Ok),
        ("warn", State::Warn),
        ("bad", State::Bad),
    ] {
        let into = std::rc::Rc::clone(&said);
        super::super::lua::api::util::put(&mut report, field, move |_, args| {
            let says = match args.first() {
                Some(Value::Str(text)) => text.to_string(),
                other => other.map(|v| v.type_name().to_string()).unwrap_or_default(),
            };
            into.borrow_mut().push((state, says));
            Ok(vec![Value::Bool(true)])
        });
    }
    let report = Value::table(report);

    for check in checks {
        if let Err(problem) = crate::lua::engine::call_here(&check, vec![report.clone()]) {
            said.borrow_mut().push((
                State::Bad,
                format!("its own health check failed: {problem}"),
            ));
        }
    }
    said.take()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plugin_with_no_check_of_its_own_says_nothing() {
        assert!(run("never-registered-anything").is_empty());
    }

    #[test]
    fn the_table_offers_health() {
        let Value::Table(built) = build() else {
            panic!("not a table")
        };
        assert!(matches!(
            built.borrow().get(&Value::str("health")),
            Value::Function(_)
        ));
    }
}

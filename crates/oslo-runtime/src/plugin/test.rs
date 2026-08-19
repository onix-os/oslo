//! `oslo.plugin.test` — the assertions a plugin writes about itself.
//!
//! ```lua
//! oslo.plugin.test("notes", function(t)
//!   t.equal(count(), 0, "a new database is empty")
//!   t.ok(oslo.db.open("notes"), "the store opens")
//! end)
//! ```
//!
//! # Why a plugin needs a harness at all
//!
//! Every plugin written for this tree so far was tested by hand, in a pty, against a temporary home
//! — which is not a thing to ask of anybody else, and is why a plugin ecosystem otherwise becomes a
//! directory of one-offs. `oslo plugin test` is what turns *I wrote a plugin* into *I maintain one*.
//!
//! **The same shape as `oslo.plugin.health`**, deliberately. A plugin author has already learnt one
//! callback-that-is-handed-a-reporter; a second one that worked differently would be a second thing
//! to learn for no gain. The difference is what they are for: a health check runs against *your*
//! machine and asks whether this plugin can work here, and a test runs against a machine with
//! nothing on it and asks whether it works at all.

use oslo_base::value::{Table, Value};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

thread_local! {
    /// Plugin name to the test bodies it registered, with the name each was given.
    static TESTS: RefCell<HashMap<String, Vec<(String, Value)>>> = RefCell::new(HashMap::new());
}

/// One assertion that did not hold, or a body that raised.
#[derive(Debug, Clone)]
pub struct Failure {
    pub says: String,
}

/// How one test came out.
#[derive(Debug, Clone)]
pub struct Outcome {
    pub name: String,
    /// How many assertions ran, so a test that asserts nothing can be told from one that passed.
    pub checked: usize,
    pub failures: Vec<Failure>,
}

impl Outcome {
    pub fn passed(&self) -> bool {
        self.failures.is_empty()
    }
}

/// Add `test` to the `oslo.plugin` table `health` builds.
pub fn install(plugin: &mut Table) {
    // oslo.plugin.test(name, function(t) … end)
    crate::lua::api::util::put(plugin, "test", |_, args| {
        let Some(Value::Str(name)) = args.first() else {
            return Err(oslo_base::value::LuaError::new(
                "oslo.plugin.test: the first argument is a name for the test".to_string(),
            ));
        };
        let Some(body @ Value::Function(_)) = args.get(1) else {
            return Err(oslo_base::value::LuaError::new(
                "oslo.plugin.test: the second argument must be a function".to_string(),
            ));
        };
        // **Keyed by the plugin, named by the test.** A plugin declares several, and a report that
        // said "notes: 3 passed" without saying which three is no use when one of them breaks.
        TESTS.with(|slot| {
            slot.borrow_mut()
                .entry(super::loading::current().map_or_else(|| name.to_string(), |it| it.plugin))
                .or_default()
                .push((name.to_string(), body.clone()))
        });
        Ok(vec![Value::Bool(true)])
    });
}

/// Run every test `plugin` registered.
pub fn run(plugin: &str) -> Vec<Outcome> {
    let tests = TESTS.with(|slot| slot.borrow().get(plugin).cloned().unwrap_or_default());
    tests
        .into_iter()
        .map(|(name, body)| run_one(name, &body))
        .collect()
}

fn run_one(name: String, body: &Value) -> Outcome {
    let failures: Rc<RefCell<Vec<Failure>>> = Rc::new(RefCell::new(Vec::new()));
    let checked = Rc::new(RefCell::new(0usize));
    let t = reporter(&failures, &checked);

    if let Err(problem) = crate::lua::engine::call_here(body, vec![t]) {
        // A body that raised is one failure, not a crash: the other tests still run, which is the
        // only way a report of five tests is worth reading when the second one is broken.
        failures.borrow_mut().push(Failure {
            says: format!("the test itself failed: {problem}"),
        });
    }
    Outcome {
        name,
        checked: *checked.borrow(),
        failures: failures.take(),
    }
}

/// The `t` a test body is handed.
fn reporter(failures: &Rc<RefCell<Vec<Failure>>>, checked: &Rc<RefCell<usize>>) -> Value {
    let mut t = Table::new();

    // t.equal(got, want, "what was being checked")
    let (into, count) = (Rc::clone(failures), Rc::clone(checked));
    crate::lua::api::util::put(&mut t, "equal", move |_, args| {
        *count.borrow_mut() += 1;
        let got = args.first().cloned().unwrap_or(Value::Nil);
        let want = args.get(1).cloned().unwrap_or(Value::Nil);
        if !got.lua_eq(&want) {
            // Both values, always. "a new database is empty: failed" sends the author back to the
            // code; "expected 0, got 3" is most of the way to the cause.
            into.borrow_mut().push(Failure {
                says: format!(
                    "{}: expected {}, got {}",
                    said(&args, 2),
                    want.to_display(),
                    got.to_display()
                ),
            });
        }
        Ok(vec![Value::Bool(true)])
    });

    // t.ok(value, "what was being checked") — anything Lua calls true
    let (into, count) = (Rc::clone(failures), Rc::clone(checked));
    crate::lua::api::util::put(&mut t, "ok", move |_, args| {
        *count.borrow_mut() += 1;
        if !args.first().map(Value::truthy).unwrap_or(false) {
            into.borrow_mut().push(Failure {
                says: format!("{}: not true", said(&args, 1)),
            });
        }
        Ok(vec![Value::Bool(true)])
    });

    // t.fail("why") — for the branch a test reached and should not have
    let into = Rc::clone(failures);
    crate::lua::api::util::put(&mut t, "fail", move |_, args| {
        into.borrow_mut().push(Failure {
            says: said(&args, 0),
        });
        Ok(vec![Value::Bool(true)])
    });

    Value::table(t)
}

/// The message argument, or a stand-in — a test that did not name what it checked still has to
/// produce a line somebody can find.
fn said(args: &[Value], at: usize) -> String {
    match args.get(at) {
        Some(Value::Str(text)) => text.to_string(),
        _ => "unnamed check".to_string(),
    }
}

/// Forget everything registered. Between one plugin's run and the next.
pub fn forget() {
    TESTS.with(|slot| slot.borrow_mut().clear());
}

#[cfg(test)]
#[path = "test/tests.rs"]
mod tests;

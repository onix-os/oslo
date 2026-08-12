//! `oslo.state` — what a config or a plugin remembers for this session and no longer.
//!
//! ```lua
//! oslo.state.set("notes.last", { key = "2026-08-12", draft = true })
//! local last = oslo.state.get("notes.last")
//! ```
//!
//! # The middle of three, and the one that was missing
//!
//! | | lives for | shape | inherited by a child |
//! |---|---|---|---|
//! | an environment variable | the session, and beyond | a string | **yes** |
//! | `oslo.state` | the session | any Lua value | no |
//! | `oslo.db` | forever | bytes on disk | no |
//!
//! Most of what a plugin wants to remember is the middle one: which shell it decided something in,
//! what it computed last prompt, whether it has already warned about something. Before this the
//! choices were an exported string — which every child process then inherits, and which a plugin
//! must encode into and out of — or a file, for a fact that does not outlive the shell.
//!
//! Neovim splits the same idea four ways (`vim.g`, `vim.b`, `vim.w`, `vim.t`) because it has buffers
//! and windows to scope to. A shell has one session, so this has one scope, and adding a second
//! later is easier than explaining a fourth today.
//!
//! **Values are Lua values, not strings.** A table stays a table, and nothing here serialises
//! anything — that is what `oslo.db` and `oslo.json` are for when a fact should outlive the shell.

use super::util::{ok, put, text};
use oslo_lua::value::{Table, Value};
use std::cell::RefCell;
use std::collections::HashMap;

thread_local! {
    static STATE: RefCell<HashMap<String, Value>> = RefCell::new(HashMap::new());
}

/// Build the `oslo.state` table.
pub fn build() -> Value {
    let mut state = Table::new();

    // oslo.state.get(key) -> value, or nil
    put(&mut state, "get", |_, args| {
        let key = text(&args, 1, "oslo.state.get")?;
        ok(STATE.with(|slot| slot.borrow().get(&key).cloned().unwrap_or(Value::Nil)))
    });

    // oslo.state.set(key, value) -> the value, so it can be assigned in one line
    put(&mut state, "set", |_, args| {
        let key = text(&args, 1, "oslo.state.set")?;
        let value = args.get(1).cloned().unwrap_or(Value::Nil);
        // **`nil` removes rather than stores.** That is what Lua does to a table field, and a
        // session's state that grew a key holding nothing would be a key `has` reports as present.
        if matches!(value, Value::Nil) {
            STATE.with(|slot| slot.borrow_mut().remove(&key));
        } else {
            STATE.with(|slot| slot.borrow_mut().insert(key, value.clone()));
        }
        ok(value)
    });

    // oslo.state.has(key) -> boolean, which `get` cannot answer for a stored `false`
    put(&mut state, "has", |_, args| {
        let key = text(&args, 1, "oslo.state.has")?;
        ok(Value::Bool(
            STATE.with(|slot| slot.borrow().contains_key(&key)),
        ))
    });

    // oslo.state.keys([prefix]) -> every key, or every key under a prefix, in order
    put(&mut state, "keys", |_, args| {
        let prefix = match args.first() {
            None | Some(Value::Nil) => String::new(),
            _ => text(&args, 1, "oslo.state.keys")?,
        };
        let mut found: Vec<String> = STATE.with(|slot| {
            slot.borrow()
                .keys()
                .filter(|key| key.starts_with(&prefix))
                .cloned()
                .collect()
        });
        found.sort();
        ok(super::util::list(found.into_iter().map(|k| Value::str(&k))))
    });

    Value::table(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(built: &Value, name: &str, args: Vec<Value>) -> Value {
        let Value::Table(table) = built else {
            panic!("not a table")
        };
        let Value::Function(f) = table.borrow().get(&Value::str(name)) else {
            panic!("no {name}")
        };
        match &*f {
            oslo_lua::value::Function::Native { call, .. } => {
                let interp = oslo_lua::Interp::new("test");
                call(&interp, args)
                    .expect(name)
                    .first()
                    .cloned()
                    .unwrap_or(Value::Nil)
            }
            _ => panic!("not native"),
        }
    }

    #[test]
    fn a_value_goes_in_and_comes_back_as_itself() {
        STATE.with(|slot| slot.borrow_mut().clear());
        let state = build();
        call(&state, "set", vec![Value::str("a"), Value::int(3)]);
        assert!(matches!(
            call(&state, "get", vec![Value::str("a")]),
            Value::Number(_)
        ));
        // A table stays a table: nothing here serialises anything.
        call(
            &state,
            "set",
            vec![Value::str("t"), Value::table(Table::new())],
        );
        assert!(matches!(
            call(&state, "get", vec![Value::str("t")]),
            Value::Table(_)
        ));
    }

    /// **A stored `false` is present**, which is exactly what `get` alone cannot tell you.
    #[test]
    fn has_answers_what_get_cannot() {
        STATE.with(|slot| slot.borrow_mut().clear());
        let state = build();
        call(&state, "set", vec![Value::str("off"), Value::Bool(false)]);
        assert!(call(&state, "has", vec![Value::str("off")]).truthy());
        assert!(!call(&state, "has", vec![Value::str("missing")]).truthy());
    }

    #[test]
    fn setting_nil_removes_the_key_the_way_a_lua_table_does() {
        STATE.with(|slot| slot.borrow_mut().clear());
        let state = build();
        call(&state, "set", vec![Value::str("a"), Value::int(1)]);
        call(&state, "set", vec![Value::str("a"), Value::Nil]);
        assert!(!call(&state, "has", vec![Value::str("a")]).truthy());
    }

    #[test]
    fn keys_come_back_in_order_and_only_under_the_prefix() {
        STATE.with(|slot| slot.borrow_mut().clear());
        let state = build();
        for key in ["notes.b", "notes.a", "other"] {
            call(&state, "set", vec![Value::str(key), Value::int(1)]);
        }
        let Value::Table(under) = call(&state, "keys", vec![Value::str("notes.")]) else {
            panic!("not a list")
        };
        let listed: Vec<String> = under
            .borrow()
            .sequence()
            .iter()
            .filter_map(|v| match v {
                Value::Str(s) => Some(s.to_string()),
                _ => None,
            })
            .collect();
        assert_eq!(listed, ["notes.a", "notes.b"]);
    }
}

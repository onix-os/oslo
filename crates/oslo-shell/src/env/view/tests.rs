//! The record, field by field.
//!
//! This is the counterpart of `lua/context.rs`'s round-trip test, and it guards the same failure:
//! a handler reading `shell.status` and silently getting nil because the field was named something
//! else. A record is a contract written in string keys, and nothing else checks it.

use super::*;

fn field(record: &Value, name: &str) -> Value {
    let Value::Table(table) = record else {
        panic!("not a table: {}", record.type_name())
    };
    table.borrow().get_str(name)
}

fn number(record: &Value, name: &str) -> i64 {
    match field(record, name) {
        Value::Number(n) => n.as_int().expect("an integer"),
        other => panic!("{name} is {}", other.type_name()),
    }
}

fn text(record: &Value, name: &str) -> String {
    match field(record, name) {
        Value::Str(s) => s.to_string(),
        other => panic!("{name} is {}", other.type_name()),
    }
}

fn all() -> Wants {
    Wants::NAMES
        .iter()
        .filter_map(|name| Wants::bit(name))
        .fold(Wants::default(), Wants::with)
}

/// **The headline.** `$?` is what a builtin could not see by any route: the `oslo.*` call raises and
/// the Lua global answers nil.
#[test]
fn the_status_is_dollar_question() {
    let mut env = Environment::new();
    env.last_status = 7;
    let shell = record(&env, "probe", Wants::default());
    assert_eq!(number(&shell, "status"), 7);
    assert!(matches!(field(&shell, "ok"), Value::Bool(false)));

    env.last_status = 0;
    let shell = record(&env, "probe", Wants::default());
    assert!(matches!(field(&shell, "ok"), Value::Bool(true)));
}

#[test]
fn the_cheap_fields_are_all_there_and_named_as_documented() {
    let mut env = Environment::new();
    env.set_var("PWD", "/somewhere", true);
    let shell = record(&env, "probe", Wants::default());
    assert_eq!(text(&shell, "name"), "probe");
    assert_eq!(text(&shell, "cwd"), "/somewhere");
    assert!(number(&shell, "pid") > 0);
    assert!(matches!(field(&shell, "interactive"), Value::Bool(_)));
    assert!(matches!(field(&shell, "flags"), Value::Str(_)));
    assert_eq!(number(&shell, "depth"), 0);
}

/// 1-based, because that is how Lua indexes and because `$1` at `positional[1]` is the only mapping
/// nobody has to remember.
#[test]
fn the_positional_parameters_are_one_based() {
    let mut env = Environment::new();
    env.set_positional(vec!["one".into(), "two".into()]);
    let shell = record(&env, "probe", Wants::default());
    assert_eq!(number(&shell, "argc"), 2);
    let Value::Table(list) = field(&shell, "positional") else {
        panic!("not a list")
    };
    let first = list.borrow().get(&Value::int(1));
    assert!(matches!(first, Value::Str(ref s) if s.as_ref() == "one"));
}

#[test]
fn pipestatus_carries_the_whole_vector() {
    let mut env = Environment::new();
    env.set_pipeline_status(vec![0, 3, 0]);
    let shell = record(&env, "probe", Wants::default());
    let Value::Table(list) = field(&shell, "pipestatus") else {
        panic!("not a list")
    };
    let second = list.borrow().get(&Value::int(2));
    assert!(matches!(second, Value::Number(n) if n.as_int() == Some(3)));
}

/// `vars` carries every variable, exported or not — which is the other thing a builtin could not
/// reach, since `oslo.env.all` shows only the exported ones and takes the lock anyway.
#[test]
fn wants_vars_carries_the_unexported_ones_too() {
    let mut env = Environment::new();
    env.set_var("SHOWN", "a", true);
    env.set_var("HIDDEN", "b", false);
    let shell = record(&env, "probe", all());
    let Value::Table(vars) = field(&shell, "vars") else {
        panic!("no vars")
    };
    assert!(matches!(vars.borrow().get_str("HIDDEN"), Value::Str(ref s) if s.as_ref() == "b"));
    let Value::Table(exported) = field(&shell, "exported") else {
        panic!("no exported")
    };
    assert!(matches!(
        exported.borrow().get_str("SHOWN"),
        Value::Bool(true)
    ));
    assert!(matches!(exported.borrow().get_str("HIDDEN"), Value::Nil));
}

#[test]
fn wants_aliases_carries_them() {
    let mut env = Environment::new();
    env.set_alias("gs", "git status");
    let shell = record(&env, "probe", all());
    let Value::Table(aliases) = field(&shell, "aliases") else {
        panic!("no aliases")
    };
    assert!(
        matches!(aliases.borrow().get_str("gs"), Value::Str(ref s) if s.as_ref() == "git status")
    );
}

/// **The justification for `wants` being opt-in at all.** A field that was not asked for is not
/// `nil` — it raises and says which declaration to change. Without this, `wants` would be a way to
/// write a silent bug.
#[test]
fn a_known_field_that_was_not_asked_for_raises() {
    let env = Environment::new();
    let shell = record(&env, "note", Wants::default());
    let Value::Table(table) = &shell else {
        panic!("not a table")
    };
    let meta = table.borrow().metatable.clone().expect("a metatable");
    let index = meta.borrow().get_str("__index");
    let Value::Function(function) = &index else {
        panic!("__index is not a function")
    };
    let oslo_base::value::Function::Held(held) = &**function else {
        panic!("not native")
    };
    let native = held
        .downcast_ref::<oslo_luavm::Native>()
        .expect("a Rust function");

    let refused = (native.call)(&Nowhere, vec![shell.clone(), Value::str("vars")]);
    let message = refused.expect_err("vars was not asked for, so it must refuse");
    assert!(
        message.to_string().contains("wants"),
        "the message should name the fix: {message}"
    );
    assert!(
        message.to_string().contains("note"),
        "the message should name the declaration: {message}"
    );
}

/// An unknown key is ordinary Lua: `nil`. A caller testing `shell.something_new` to find out
/// whether this build has it is doing a reasonable thing and must not be punished for it.
#[test]
fn an_unknown_field_is_still_nil() {
    let env = Environment::new();
    let shell = record(&env, "note", Wants::default());
    let Value::Table(table) = &shell else {
        panic!("not a table")
    };
    let meta = table.borrow().metatable.clone().expect("a metatable");
    let index = meta.borrow().get_str("__index");
    let Value::Function(function) = &index else {
        panic!("not a function")
    };
    let oslo_base::value::Function::Held(held) = &**function else {
        panic!("not native")
    };
    let native = held
        .downcast_ref::<oslo_luavm::Native>()
        .expect("a Rust function");
    let answered = (native.call)(&Nowhere, vec![shell.clone(), Value::str("invented")])
        .expect("an unknown key does not raise");
    assert!(matches!(answered.first(), Some(Value::Nil) | None));
}

/// Asking for everything leaves no refusals, so the record is a plain table.
#[test]
fn a_record_that_gathered_everything_needs_no_metatable() {
    let env = Environment::new();
    let shell = record(&env, "probe", all());
    let Value::Table(table) = &shell else {
        panic!("not a table")
    };
    assert!(table.borrow().metatable.is_none());
}

#[test]
fn an_unknown_wants_name_has_no_bit() {
    assert!(Wants::bit("vars").is_some());
    assert!(Wants::bit("variables").is_none());
}

/// A host with no interpreter, for calling the metatable's `__index` directly.
pub(super) struct Nowhere;

impl oslo_luavm::Host for Nowhere {
    fn global(&self, _name: &str) -> Value {
        Value::Nil
    }
    fn set_global(&self, _name: &str, _value: Value) {}
    fn set_field(&self, _path: &[&str], _value: Value) -> bool {
        false
    }
    fn chunk(&self) -> String {
        "test".to_string()
    }
    fn call(&self, _f: &Value, _args: Vec<Value>) -> oslo_base::value::LuaResult<Vec<Value>> {
        Err(LuaError::new("no interpreter"))
    }
    fn eval(&self, _s: &str, _c: &str) -> oslo_base::value::LuaResult<Vec<Value>> {
        Err(LuaError::new("no interpreter"))
    }
    fn load(&self, _s: &str, _c: &str) -> oslo_base::value::LuaResult<Value> {
        Err(LuaError::new("no interpreter"))
    }
}

//! Calling a binding from a unit test, with no VM behind it.
//!
//! A namespace is *built* without an engine — that is the point of [`native`](super::native), which
//! wraps a Rust closure in a shell value carrying an opaque payload. A unit test wants to call one
//! back out again, and it wants to do so without standing up an interpreter for the sake of two
//! arguments. That takes two pieces: reaching the closure through `Function::Held`, and a [`Host`]
//! for it to be handed.
//!
//! [`Nowhere`] is that host. Every method is the honest answer for "there is no VM here" — a global
//! is nil, a write goes nowhere, a call is refused. A binding that genuinely needs the engine is
//! not one a unit test can exercise, and it says so rather than pretending.

use oslo_base::value::{Function, LuaError, LuaResult, Value};
use oslo_luavm::{Host, Native};

/// A [`Host`] with no interpreter behind it.
pub(crate) struct Nowhere;

impl Host for Nowhere {
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

    fn call(&self, _function: &Value, _args: Vec<Value>) -> LuaResult<Vec<Value>> {
        Err(LuaError::new("no interpreter in this test"))
    }

    fn eval(&self, _source: &str, _chunk: &str) -> LuaResult<Vec<Value>> {
        Err(LuaError::new("no interpreter in this test"))
    }

    fn load(&self, _source: &str, _chunk: &str) -> LuaResult<Value> {
        Err(LuaError::new("no interpreter in this test"))
    }
}

/// Call the Rust function `value` carries.
pub(crate) fn call(value: &Value, args: Vec<Value>) -> LuaResult<Vec<Value>> {
    let Value::Function(function) = value else {
        panic!("not a function: {}", value.type_name())
    };
    let Function::Held(held) = &**function else {
        panic!("not callable from here")
    };
    let native = held
        .downcast_ref::<Native>()
        .expect("not a Rust function — a Lua one cannot be called without a VM");
    (native.call)(&Nowhere, args)
}

/// Call it and take the first value, or `nil`.
pub(crate) fn first(value: &Value, args: Vec<Value>) -> Value {
    call(value, args)
        .expect("the call raised")
        .into_iter()
        .next()
        .unwrap_or(Value::Nil)
}

/// What `handle.name` answers — through `__index`, which is where a handle keeps its verbs.
///
/// See [`super::super::handle`]. A plain table is read directly, so this works either way.
pub(crate) fn field(handle: &Value, name: &str) -> Value {
    let Value::Table(table) = handle else {
        panic!("not a table: {}", handle.type_name())
    };
    let direct = table.borrow().get_str(name);
    if !matches!(direct, Value::Nil) {
        return direct;
    }
    let meta = table.borrow().metatable.clone();
    match meta.map(|meta| meta.borrow().get_str("__index")) {
        Some(Value::Table(index)) => index.borrow().get_str(name),
        _ => Value::Nil,
    }
}

/// Call `handle:name(...)` — the arguments a `:` call makes, `self` and all.
pub(crate) fn method(handle: &Value, name: &str, mut args: Vec<Value>) -> LuaResult<Vec<Value>> {
    let verb = field(handle, name);
    args.insert(0, handle.clone());
    call(&verb, args)
}

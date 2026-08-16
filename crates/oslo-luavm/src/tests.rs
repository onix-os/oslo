//! The boundary, tested in both directions.
//!
//! Conversion is checked as a round trip rather than field by field, because what matters is that a
//! value survives the crossing — and the two failures that would matter most, a native that cannot
//! be called and a Lua function that cannot be held, are each their own test.

use super::Engine;
use super::convert::{from_lua, into_lua};
use super::host::{Host, Native};
use luna::Lua;
use oslo_base::value::{Function, LuaError, Number, Table, Value};
use std::cell::RefCell;
use std::rc::Rc;

/// Build the shell value that carries a Rust function.
fn native(
    name: &'static str,
    f: impl Fn(&dyn Host, Vec<Value>) -> Result<Vec<Value>, LuaError> + 'static,
) -> Value {
    Value::Function(Rc::new(Function::Held(Rc::new(Native {
        name,
        call: Box::new(f),
    }))))
}

#[test]
fn a_scalar_survives_the_round_trip() {
    let mut lua = Lua::core();
    lua.enter(|ctx| {
        for original in [
            Value::Nil,
            Value::Bool(true),
            Value::Number(Number::Int(-7)),
            Value::Number(Number::Float(0.5)),
            Value::str("hello"),
        ] {
            let back = from_lua(ctx, into_lua(ctx, &original));
            assert_eq!(
                format!("{original:?}"),
                format!("{back:?}"),
                "a {} did not survive",
                original.type_name()
            );
        }
    });
}

/// Tables compare as sets: luna's hash part is `ahash`, seeded per process, so iteration order
/// differs between runs and asserting on it would be a flake rather than a check.
#[test]
fn a_table_survives_the_round_trip() {
    let mut lua = Lua::core();
    let mut table = Table::new();
    table.set(Value::str("name"), Value::str("oslo"));
    table.set(Value::int(1), Value::str("first"));
    let original = Value::Table(Rc::new(RefCell::new(table)));

    lua.enter(|ctx| {
        let Value::Table(back) = from_lua(ctx, into_lua(ctx, &original)) else {
            panic!("a table came back as something else");
        };
        let mut pairs: Vec<String> = back
            .borrow()
            .pairs()
            .iter()
            .map(|(k, v)| format!("{k:?}={v:?}"))
            .collect();
        pairs.sort();
        assert_eq!(pairs.len(), 2, "got {pairs:?}");
        assert!(pairs.iter().any(|p| p.contains("oslo")), "got {pairs:?}");
    });
}

/// **A Rust function registered as a global is callable from Lua.**
///
/// This is the whole binding surface in one test: two hundred callables reach the VM by exactly
/// this path, so if the wrapper drops arguments, loses results, or is not callable at all, this
/// fails before any of them are ported.
#[test]
fn lua_can_call_a_rust_function() {
    let engine = Engine::new();
    engine.set_global(
        "double",
        native("double", |_host, args| {
            let n = args
                .first()
                .and_then(Value::as_number)
                .and_then(|n| n.as_int())
                .ok_or_else(|| LuaError::new("double: want a number"))?;
            Ok(vec![Value::int(n * 2)])
        }),
    );

    let returned = engine
        .eval("return double(21)", "=test")
        .expect("the chunk runs");
    assert_eq!(
        format!("{:?}", returned.first()),
        format!("{:?}", Some(&Value::int(42))),
        "the native did not answer through the VM"
    );
}

/// A native's failure arrives as a Lua error the script can catch and read.
#[test]
fn a_native_failure_reaches_pcall_as_a_message() {
    let engine = Engine::new();
    engine.set_global(
        "boom",
        native("boom", |_host, _args| {
            Err(LuaError::new("boom: nothing worked"))
        }),
    );

    let returned = engine
        .eval("local ok, err = pcall(boom) return ok, err", "=test")
        .expect("the chunk runs");
    assert_eq!(
        format!("{:?}", returned.first()),
        format!("{:?}", Some(&Value::Bool(false))),
        "pcall reported success for a native that failed"
    );
    let message = format!("{:?}", returned.get(1));
    assert!(
        message.contains("nothing worked"),
        "the message did not survive: {message}"
    );
}

/// **A Lua function can be held by Rust and called later.**
///
/// Hooks, completers, timers and the prompt handler all work this way: the script hands a function
/// over, the shell stores it in an ordinary struct, and calls it turns later. It only works because
/// the value is rooted in the VM's registry on the way out — an unrooted one would be collected.
#[test]
fn rust_can_hold_and_call_a_lua_function() {
    let engine = Engine::new();
    let held = engine
        .eval("return function(x) return x .. ' held' end", "=test")
        .expect("the chunk runs")
        .into_iter()
        .next()
        .expect("a function came back");
    assert!(
        matches!(held, Value::Function(_)),
        "what came back was not a function: {held:?}"
    );

    // A collection between the two, so the test fails if the function was never rooted.
    engine
        .eval("for _ = 1, 2000 do local _ = {} end", "=test")
        .expect("the churn runs");

    let answered = engine
        .call_function(&held, vec![Value::str("value")])
        .expect("the held function is still callable");
    assert_eq!(
        answered.first().map(|v| v.to_display()),
        Some("value held".to_string()),
        "got {answered:?}"
    );
}

/// A native reading a global reads the one the script set.
#[test]
fn a_native_sees_the_scripts_globals() {
    let engine = Engine::new();
    engine.set_global(
        "peek",
        native("peek", |host, _args| Ok(vec![host.global("marker")])),
    );

    let returned = engine
        .eval("marker = 'set-by-script' return peek()", "=test")
        .expect("the chunk runs");
    assert_eq!(
        returned.first().map(|v| v.to_display()),
        Some("set-by-script".to_string()),
        "got {returned:?}"
    );
}

/// An unfinished line asks for more; a wrong one does not.
#[test]
fn an_incomplete_chunk_is_told_from_a_broken_one() {
    for unfinished in ["if true then", "local x = {", "function f()", "for i = 1, 2 do"] {
        assert!(
            !super::is_complete(unfinished),
            "`{unfinished}` should have asked for another line"
        );
    }
    for finished in ["x = 1", "if true then end", "print('hi')"] {
        assert!(super::is_complete(finished), "`{finished}` is a whole chunk");
    }
    // A real mistake must run and report, not hang waiting for input that cannot fix it.
    assert!(
        super::is_complete("x = = 2"),
        "a broken line was mistaken for an unfinished one"
    );
}

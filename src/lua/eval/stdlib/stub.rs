//! The parts of Lua oslo does not implement — present, named, and erroring.
//!
//! The alternative is leaving them `nil`, and the difference shows the first time a script written
//! for real Lua runs here:
//!
//! ```text
//! nil:            dkjson.lua:412: attempt to index a nil value (global 'coroutine')
//! this module:    dkjson.lua:412: coroutine.wrap is not implemented in oslo's Lua
//! ```
//!
//! The first sends the reader looking for a typo. The second says what happened, and is a concrete
//! request for the next thing to build.

use super::super::value::Value;
use super::super::{Interp, LuaError, LuaResult};
use super::{module, native};

/// A function that exists only to say it does not.
fn missing(name: &'static str) -> Value {
    native(
        name,
        move |_: &mut Interp, _: Vec<Value>| -> LuaResult<Vec<Value>> {
            Err(LuaError::new(format!(
                "{name} is not implemented in oslo's Lua"
            )))
        },
    )
}

/// Build a namespace whose every member refuses.
fn refusing(namespace: &str, names: &[&'static str]) -> Value {
    module(
        names
            .iter()
            .map(|name| {
                // Leaked so the message can name the full path with a `'static` string, which is
                // what `native` takes. One small allocation per stub, once per process.
                let full: &'static str = Box::leak(format!("{namespace}.{name}").into_boxed_str());
                (*name, missing(full))
            })
            .collect(),
    )
}

pub fn install(interp: &mut Interp) {
    // Coroutines need the interpreter to be suspendable mid-call, which a tree-walker running on
    // the Rust stack cannot be without either threads or a bytecode VM. Both were considered and
    // declined; see PLAN-LUA.md.
    interp.set_global(
        "coroutine",
        refusing(
            "coroutine",
            &[
                "create",
                "resume",
                "yield",
                "status",
                "wrap",
                "isyieldable",
                "running",
                "close",
            ],
        ),
    );

    // `debug` reaches into interpreter internals this evaluator does not expose. `traceback` is
    // the one scripts genuinely rely on, and it answers rather than refusing.
    let debug = module(vec![
        ("traceback", native("debug.traceback", traceback)),
        ("getinfo", missing("debug.getinfo")),
        ("sethook", missing("debug.sethook")),
        ("getlocal", missing("debug.getlocal")),
        ("setlocal", missing("debug.setlocal")),
        ("getupvalue", missing("debug.getupvalue")),
        ("setupvalue", missing("debug.setupvalue")),
        ("setmetatable", missing("debug.setmetatable")),
        ("getmetatable", missing("debug.getmetatable")),
    ]);
    interp.set_global("debug", debug);

    // UTF-8 is a real gap rather than a design decision: oslo's strings are byte strings, and
    // these functions are the ones a script uses when it needs to know better.
    interp.set_global(
        "utf8",
        refusing("utf8", &["char", "codepoint", "len", "offset", "codes"]),
    );
}

/// `debug.traceback` — the message, and a note that frames are not walkable here.
fn traceback(_: &mut Interp, args: Vec<Value>) -> LuaResult<Vec<Value>> {
    let message = match args.first() {
        Some(Value::Str(s)) => s.to_string(),
        Some(Value::Nil) | None => String::new(),
        // Lua returns a non-string argument untouched, which is how error objects survive being
        // passed through `xpcall(f, debug.traceback)`.
        Some(other) => return Ok(vec![other.clone()]),
    };
    Ok(vec![Value::str(if message.is_empty() {
        "stack traceback:".to_string()
    } else {
        format!("{message}\nstack traceback:")
    })])
}

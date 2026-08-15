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

use super::super::value::{Table, Value};
use super::super::{Interp, LuaError, LuaResult};
use super::{module, native};
use std::rc::Rc;

/// A function that exists only to say it does not.
pub(super) fn missing(name: &'static str) -> Value {
    native(
        name,
        move |_: &Interp, _: Vec<Value>| -> LuaResult<Vec<Value>> {
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

pub fn install(interp: &Interp) {
    // Coroutines need the interpreter to be suspendable mid-call, which a tree-walker running on
    // the Rust stack cannot be without either threads or a bytecode VM. Both were considered and
    // declined; see PLAN.md.
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
        with(
            refusing("utf8", &["char", "codepoint", "len", "offset", "codes"]),
            // A constant, not a function: the pattern that matches one UTF-8 character. It costs
            // nothing to give and a script that only wanted this had no reason to be refused.
            &[(
                "charpattern",
                Value::str("[\0-\u{7F}\u{C2}-\u{FD}][\u{80}-\u{BF}]*"),
            )],
        ),
    );

    io(interp);
}

/// `io`, with the parts a shell script actually uses and pointers for the rest.
///
/// `io.write` and `io.read` are here because they are how a Lua script talks to the terminal.
/// File *handles* are not: they need `__close` and a userdata type this evaluator does not have,
/// and `oslo.fs.read`/`.write`/`.lines` cover what a shell script does with a file anyway.
///
/// `io.popen` is refused by name rather than left nil, because in real Lua it runs a command
/// through `/bin/sh` — someone else's shell, from inside this one, and nothing at all on a system
/// where oslo is the only shell installed.
fn io(interp: &Interp) {
    let library = module(vec![
        ("write", native("io.write", write)),
        ("read", native("io.read", read)),
        (
            "popen",
            native("io.popen", |_: &Interp, _: Vec<Value>| {
                Err(LuaError::new(
                    "io.popen runs its argument through /bin/sh, which is not this shell; \
                     use oslo.run{..., capture = true}",
                ))
            }),
        ),
        ("open", missing("io.open")),
        ("close", missing("io.close")),
        ("lines", missing("io.lines")),
        ("input", missing("io.input")),
        ("output", missing("io.output")),
        ("tmpfile", missing("io.tmpfile")),
        ("type", native("io.type", handle_type)),
        ("flush", native("io.flush", flush)),
        // **The three standard streams, as things you can write to.** `io.stderr:write(msg)` is
        // how a Lua program complains, and it was `attempt to index a nil value` — the exact
        // failure the rule at the top of this file exists to prevent. They are plain tables rather
        // than file handles, which is enough for `write`, `flush` and `close`; anything that needs
        // a real handle says so by name.
        ("stdout", stream(Stream::Out)),
        ("stderr", stream(Stream::Err)),
        ("stdin", stream(Stream::In)),
    ]);
    interp.set_global("io", library);
}

/// Which stream a handle stands for.
#[derive(Clone, Copy, PartialEq)]
enum Stream {
    Out,
    Err,
    In,
}

impl Stream {
    fn name(self) -> &'static str {
        match self {
            Stream::Out => "io.stdout",
            Stream::Err => "io.stderr",
            Stream::In => "io.stdin",
        }
    }
}

/// One standard stream as a table with the methods a script actually calls on one.
///
/// `f:write(...)` passes the table as the first argument, so every method here drops it. `write`
/// answers the handle, as Lua's does, so `io.stderr:write(a):write(b)` chains.
fn stream(which: Stream) -> Value {
    let handle = Table::new();
    let handle = Rc::new(std::cell::RefCell::new(handle));
    let name = which.name();
    handle
        .borrow_mut()
        .set(Value::str("oslo_stream"), Value::str(name));
    let me = handle.clone();
    handle.borrow_mut().set(
        Value::str("write"),
        native(name, move |interp: &Interp, args: Vec<Value>| {
            use std::io::Write;
            if which == Stream::In {
                return Err(LuaError::new("io.stdin: cannot write to standard input"));
            }
            let mut text = String::new();
            // The first argument is the handle itself, from the `:` call.
            for value in args.iter().skip(1) {
                text.push_str(&super::super::ops::tostring(interp, value)?);
            }
            let written = match which {
                Stream::Err => std::io::stderr().lock().write_all(text.as_bytes()),
                _ => std::io::stdout().lock().write_all(text.as_bytes()),
            };
            written.map_err(|e| LuaError::new(format!("{name}: {e}")))?;
            Ok(vec![Value::Table(me.clone())])
        }),
    );
    let flushing = which;
    handle.borrow_mut().set(
        Value::str("flush"),
        native(name, move |_: &Interp, _: Vec<Value>| {
            use std::io::Write;
            let _ = match flushing {
                Stream::Err => std::io::stderr().flush(),
                _ => std::io::stdout().flush(),
            };
            Ok(Vec::new())
        }),
    );
    // Closing a standard stream is a no-op here rather than an error: a script that tidies up
    // after itself should not fail for being tidy.
    handle.borrow_mut().set(
        Value::str("close"),
        native(name, |_: &Interp, _: Vec<Value>| Ok(Vec::new())),
    );
    handle.borrow_mut().set(
        Value::str("read"),
        missing(match which {
            Stream::In => "io.stdin:read",
            Stream::Out => "io.stdout:read",
            Stream::Err => "io.stderr:read",
        }),
    );
    handle.borrow_mut().set(
        Value::str("lines"),
        missing(match which {
            Stream::In => "io.stdin:lines",
            Stream::Out => "io.stdout:lines",
            Stream::Err => "io.stderr:lines",
        }),
    );
    Value::Table(handle)
}

/// `io.type(x)` — `"file"` for one of the three streams above, nil for anything else.
///
/// Never `"closed file"`: none of them can be closed. A script uses this to tell a handle from a
/// string before calling a method on it, which is exactly what it answers.
fn handle_type(_: &Interp, args: Vec<Value>) -> LuaResult<Vec<Value>> {
    let Some(Value::Table(t)) = args.first() else {
        return Ok(vec![Value::Nil]);
    };
    let tagged = matches!(t.borrow().get(&Value::str("oslo_stream")), Value::Str(_));
    Ok(vec![match tagged {
        true => Value::str("file"),
        false => Value::Nil,
    }])
}

/// `io.flush` — push standard output out now.
fn flush(_: &Interp, _: Vec<Value>) -> LuaResult<Vec<Value>> {
    use std::io::Write;
    let _ = std::io::stdout().flush();
    Ok(Vec::new())
}

/// A namespace with extra members added to it.
fn with(namespace: Value, extra: &[(&str, Value)]) -> Value {
    if let Value::Table(t) = &namespace {
        for (name, value) in extra {
            t.borrow_mut().set(Value::str(name), value.clone());
        }
    }
    namespace
}

/// `io.write` — the arguments, concatenated, with no separator and no newline.
fn write(interp: &Interp, args: Vec<Value>) -> LuaResult<Vec<Value>> {
    use std::io::Write;
    let mut line = String::new();
    for value in &args {
        line.push_str(&super::super::ops::tostring(interp, value)?);
    }
    std::io::stdout()
        .lock()
        .write_all(line.as_bytes())
        .map_err(|e| LuaError::new(format!("io.write: {e}")))?;
    Ok(Vec::new())
}

/// `io.read` — one line from standard input, or nil at end of file.
///
/// Only the `"l"`/`"*l"` format, which is what every prompt-a-user script uses. `"n"` and `"a"`
/// would each need their own answer about what happens at EOF, and neither has come up.
fn read(_: &Interp, args: Vec<Value>) -> LuaResult<Vec<Value>> {
    if let Some(Value::Str(format)) = args.first()
        && !matches!(&**format, "l" | "*l" | "L" | "*L")
    {
        return Err(LuaError::new(format!(
            "io.read({format:?}) is not implemented in oslo's Lua; only line reads are"
        )));
    }
    let mut line = String::new();
    match std::io::BufRead::read_line(&mut std::io::stdin().lock(), &mut line) {
        Ok(0) => Ok(vec![Value::Nil]),
        // `L` keeps the newline, `l` drops it.
        Ok(_) => {
            let keep = matches!(args.first(), Some(Value::Str(f)) if matches!(&**f, "L" | "*L"));
            if !keep {
                line.truncate(line.trim_end_matches('\n').len());
            }
            Ok(vec![Value::str(line)])
        }
        Err(e) => Err(LuaError::new(format!("io.read: {e}"))),
    }
}

/// `debug.traceback` — the message, and a note that frames are not walkable here.
fn traceback(_: &Interp, args: Vec<Value>) -> LuaResult<Vec<Value>> {
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

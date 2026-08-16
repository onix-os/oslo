//! Converting between the shell's value and the VM's, which is the whole boundary.
//!
//! oslo's [`oslo_base::value::Value`] is owned and lives anywhere; luna's `Value<'gc>` is a handle
//! into a collected heap and lives only inside `Lua::enter`. Everything the shell hands to Lua goes
//! one way through here, and everything Lua answers comes back the other, so the lifetime stops at
//! this file rather than spreading through forty crates that have no VM in them.
//!
//! # Functions cross too, and that is what makes the port small
//!
//! [`Function::Held`](oslo_base::value::Function::Held) is an `Rc<dyn Any>` the shell never looks
//! inside. Two different things travel in it, and this file is the only place that knows which:
//!
//! - a [`Native`] — a Rust closure going *in*, wrapped as a luna callback;
//! - a [`StashedFunction`] — a Lua function coming *out*, rooted so the collector keeps it.
//!
//! That is why two hundred registered callables did not have to be rewritten against a VM type.
//! They build shell values and answer shell values; the engine boundary is here.

use crate::host::{CallbackHost, Native};
use luna::{
    Callback, CallbackReturn, Context, Function as LunaFn, StashedFunction, String as LunaStr,
    Table, Value, Variadic,
};
use oslo_base::value::{Function, LuaError, Number, Value as Own};
use std::cell::RefCell;
use std::rc::Rc;

/// The shell's value, as something the VM can hold.
pub fn into_lua<'gc>(ctx: Context<'gc>, value: &Own) -> Value<'gc> {
    match value {
        Own::Nil => Value::Nil,
        Own::Bool(b) => Value::Boolean(*b),
        // Lua's own split, kept: `3` and `3.0` compare equal but format differently and divide
        // differently, which is why `oslo_base` keeps two variants rather than one `f64`.
        Own::Number(Number::Int(i)) => Value::Integer(*i),
        Own::Number(Number::Float(f)) => Value::Number(*f),
        // Copied, not borrowed: `IntoValue for &str` wants `&'static`, and the shell's strings
        // are `Rc<str>` owned by a value that outlives nothing in particular.
        Own::Str(s) => Value::String(LunaStr::from_slice(&ctx, s.as_bytes())),
        Own::Table(table) => {
            let out = Table::new(&ctx);
            for (key, value) in table.borrow().pairs() {
                // A `set` only fails on a nil or NaN key. `pairs` yields neither: a nil key
                // cannot be stored, and a NaN one never compares equal to itself so it could
                // not have been either.
                let _ = out.set(ctx, into_lua(ctx, &key), into_lua(ctx, &value));
            }
            // **The metatable crosses too, or three headline surfaces are silently dead.** `sh` is
            // an *empty* table whose whole API is a `__index` that manufactures a command wrapper;
            // `oslo.prompt` and `oslo.theme.styles` are empty tables whose `__newindex` is what
            // files a prompt handler or defines a style. Copying only the entries delivers `{}` to
            // the VM, and `sh.ls(…)` becomes "attempt to call a nil value" while
            // `oslo.prompt.left = f` succeeds at doing nothing.
            if let Some(meta) = &table.borrow().metatable
                && let Value::Table(meta) = into_lua(ctx, &Own::Table(Rc::clone(meta)))
            {
                out.set_metatable(&ctx, Some(meta));
            }
            Value::Table(out)
        }
        Own::Function(f) => function_into_lua(ctx, f),
    }
}

/// A callable crossing into the VM: a Rust one gets wrapped, a Lua one gets handed back.
fn function_into_lua<'gc>(ctx: Context<'gc>, f: &Rc<Function>) -> Value<'gc> {
    let Function::Held(held) = &**f else {
        // `Native { name }` carries no body — it exists so a value can *say* it is a function
        // without an engine in scope. Nothing registers one, so nothing can call one.
        return Value::Nil;
    };
    if let Some(stashed) = held.downcast_ref::<StashedFunction>() {
        return Value::Function(ctx.fetch(stashed));
    }
    match held.clone().downcast::<Native>() {
        Ok(native) => Value::Function(LunaFn::Callback(wrap(ctx, native))),
        Err(_) => Value::Nil,
    }
}

/// A Rust function as a luna callback.
///
/// The closure captures the `Rc` itself rather than an index into a registry, which is what
/// [`Callback::from_fn`] permits and what keeps the native's lifetime tied to the value holding it.
fn wrap<'gc>(ctx: Context<'gc>, native: Rc<Native>) -> Callback<'gc> {
    Callback::from_fn(&ctx, move |ctx, _exec, mut stack| {
        let args: Vec<Own> = stack.drain(..).map(|v| from_lua(ctx, v)).collect();
        let host = CallbackHost::new(ctx);
        // Published for the duration of the call, so shell code several Rust frames down can call
        // back into Lua — a registered tool, a registered builtin, a hook fired mid-command.
        match crate::host::while_running(&host, || (native.call)(&host, args)) {
            Ok(values) => {
                // `Variadic`, or a `Vec` would arrive as one table: returning two values and
                // returning a list of two are different things, and every multi-return binding
                // depends on the difference.
                let out: Vec<Value<'gc>> = values.iter().map(|v| into_lua(ctx, v)).collect();
                stack.replace(ctx, Variadic(out));
                Ok(CallbackReturn::Return)
            }
            Err(error) => Err(raise(ctx, &error)),
        }
    })
}

/// A shell error as the value luna unwinds with.
///
/// Rendered rather than boxed: `pcall` hands the error *value* to the script, and the idiom for
/// reading one is `message:match(":(%d+):")`. A Rust error object would arrive as userdata the
/// script cannot match on, so what travels is the same `chunk:line: message` string the tree
/// walker produced.
pub fn raise<'gc>(ctx: Context<'gc>, error: &LuaError) -> luna::Error<'gc> {
    // An exit is not a failure, and the status cannot ride in a string. It waits beside the VM
    // while the error unwinds, and the engine reports it instead of the message.
    if let Some(status) = error.exit {
        crate::note_exit(status);
    }
    let text = error.value_string(error.chunk.as_deref().unwrap_or("?"));
    luna::Error::from_value(Value::String(LunaStr::from_slice(&ctx, text.as_bytes())))
}

/// The VM's value, as something the shell can keep.
///
/// **Key order is not preserved.** oslo's table remembers the order keys were inserted, which is
/// what makes `to json` and `to text` print a row's columns the way the document had them.
/// luna's hash part has no order to give back — it is `ahash`, seeded per process, so two runs
/// disagree. A row that goes into Lua and comes back out therefore loses its column order. That
/// is a real gap for the structured pipeline and is recorded in `PLAN-LUA-VM.md`; the array part
/// (`1..n`) is unaffected, because it is a vector.
pub fn from_lua<'gc>(ctx: Context<'gc>, value: Value<'gc>) -> Own {
    from_lua_within(ctx, value, 0)
}

/// How deep a table may nest before the conversion gives up.
///
/// A depth cap rather than a visited-set, because the shell's values are trees by construction —
/// what this guards against is a *cyclic* table reaching the conversion at all, and a script can
/// make one in a line.
const MAX_DEPTH: usize = 64;

fn from_lua_within<'gc>(ctx: Context<'gc>, value: Value<'gc>, depth: usize) -> Own {
    match value {
        Value::Nil => Own::Nil,
        Value::Boolean(b) => Own::Bool(b),
        Value::Integer(i) => Own::Number(Number::Int(i)),
        Value::Number(f) => Own::Number(Number::Float(f)),
        Value::String(s) => Own::str(String::from_utf8_lossy(s.as_bytes())),
        // Rooted in the VM's registry, so the collector keeps it while the shell holds it. This is
        // how a hook, a completer or a prompt handler survives past the call that installed it.
        Value::Function(f) => Own::Function(Rc::new(Function::Held(Rc::new(ctx.stash(f))))),
        Value::Table(t) if depth < MAX_DEPTH => {
            let mut out = oslo_base::value::Table::new();
            for (key, value) in t.iter() {
                out.set(
                    from_lua_within(ctx, key, depth + 1),
                    from_lua_within(ctx, value, depth + 1),
                );
            }
            if let Some(meta) = t.metatable()
                && let Own::Table(meta) = from_lua_within(ctx, Value::Table(meta), depth + 1)
            {
                out.metatable = Some(meta);
            }
            Own::Table(Rc::new(RefCell::new(out)))
        }
        // Deeper than the cap, or a thread, or userdata: nothing the shell's value type has a
        // home for.
        _ => Own::Nil,
    }
}

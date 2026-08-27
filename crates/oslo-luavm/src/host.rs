//! What a registered Rust function is handed, and what it may ask of the VM.
//!
//! Two hundred callables take this and two use it. That ratio is the whole design: a binding gets
//! shell values in and answers shell values out, so it needs nothing from the engine, and the
//! handful that genuinely reach back into Lua go through a named interface rather than through the
//! interpreter's guts.
//!
//! # Why there are two implementations
//!
//! luna is *stackless*: the VM's call stack lives in its own heap, not on Rust's, which is how it
//! gets coroutines and unbounded recursion. The cost is that a callback cannot re-enter the VM —
//! stepping the executor needs mutable access to the arena, and a running callback is already
//! inside it. So [`CallbackHost`], the one a native sees mid-call, can read and write globals but
//! answers [`Host::call`] with a refusal, while [`Engine`](crate::Engine) — the one Rust holds at
//! the top level — can do everything. A native that must call a Lua function is written as a
//! sequence instead; see `CallbackReturn::Call`.

use crate::convert::{from_lua, into_lua};
use luna::{Context, String as LunaStr, Value};
use oslo_base::value::{LuaError, LuaResult, Value as Own};
use std::cell::Cell;
use std::ptr::NonNull;

/// The Rust side of a builtin: arguments in, values out, or a Lua error.
///
/// Boxed rather than a plain `fn` pointer because several are closures over state — a completer
/// captures the registry it answers from, a timer its handle.
pub type NativeFn = Box<dyn Fn(&dyn Host, Vec<Own>) -> LuaResult<Vec<Own>>>;

/// A named Rust function, as it travels inside a shell value.
///
/// Reached only through [`Function::Held`](oslo_base::value::Function::Held), which is an
/// `Rc<dyn Any>` — so `oslo-base` can carry a callable without knowing an engine exists.
pub struct Native {
    /// What an error message has to be able to say.
    pub name: &'static str,
    pub call: NativeFn,
}

/// What a native may ask of the engine that is calling it.
pub trait Host {
    /// A global by name, or `nil`.
    fn global(&self, name: &str) -> Own;

    /// Set a global.
    fn set_global(&self, name: &str, value: Own);

    /// Set a nested field, walking from the globals: `["package", "preload", "oslo.ui"]`.
    ///
    /// **Because a table read out of the VM is a copy.** The tree walker's tables were the
    /// interpreter's own `Rc`, so fetching `package.preload` and inserting into it was visible to
    /// the next `require`. A VM table cannot leave the collector, so what [`global`](Self::global)
    /// answers is a snapshot and writing to it changes nothing. Anything that means to reach back
    /// into a live table goes through here, where the write happens inside the arena.
    ///
    /// Answers whether the path existed and the write landed.
    fn set_field(&self, path: &[&str], value: Own) -> bool;

    /// Read a nested field, walking from the globals, converting only the leaf.
    ///
    /// The read counterpart to [`set_field`](Self::set_field), and the answer to the same trap:
    /// [`global`](Self::global) hands back a deep copy, so a caller holding one is holding a
    /// snapshot. A real engine walks the live tables every time it is asked.
    ///
    /// Defaulted to walking what `global` answers, which is correct — the copy is of the state at
    /// *this* moment — and merely slower. That is the right trade for the hosts that have no arena
    /// to walk: a probe and the test doubles.
    fn field(&self, path: &[&str]) -> Own {
        let Some((first, rest)) = path.split_first() else {
            return Own::Nil;
        };
        let mut value = self.global(first);
        for name in rest {
            let Own::Table(table) = value else {
                return Own::Nil;
            };
            let next = table.borrow().get_str(name);
            value = next;
        }
        value
    }

    /// Lift the members of a global table to globals of their own — `oslo.fs` also as `fs`.
    ///
    /// Defaulted to doing nothing, because it is a convenience of the real engine rather than
    /// something every host has to have an answer for: the tree walker and the test doubles
    /// implement this trait too, and a shorthand they do not offer is not a failure.
    fn flatten_namespace(&self, _table: &str) {}

    /// What the running source is called, for the position on an error.
    fn chunk(&self) -> String;

    /// Call a Lua function.
    ///
    /// Refused while a native is running — see the module note on stacklessness.
    fn call(&self, function: &Own, args: Vec<Own>) -> LuaResult<Vec<Own>>;

    /// Run a chunk and answer what it returned.
    ///
    /// Used at *install* time, where a binding whose shape needs a Lua-side wrapper compiles one
    /// and keeps it.
    fn eval(&self, source: &str, chunk: &str) -> LuaResult<Vec<Own>>;

    /// Compile a chunk without running it, answering it as a callable value.
    fn load(&self, source: &str, chunk: &str) -> LuaResult<Own>;
}

/// Read a nested field, walking from the globals: `["oslo", "completion", "for_command", "git"]`.
///
/// **The counterpart to [`set_field_in`], and for the same reason.** What [`Host::global`] answers
/// is a deep copy taken at that moment, so a caller that fetched a table at startup went on
/// consulting a snapshot for the rest of the session — anything registered later was invisible to
/// it. Walking here reads the live tables and converts only the *leaf*, which for a function is a
/// stash handle rather than a copy of anything.
pub(crate) fn field_in<'gc>(ctx: Context<'gc>, path: &[&str]) -> Own {
    let Some((last, walked)) = path.split_last() else {
        return Own::Nil;
    };
    let mut table = ctx.globals();
    for name in walked {
        let key = Value::String(LunaStr::from_slice(&ctx, name.as_bytes()));
        match table.get_value(ctx, key) {
            Value::Table(next) => table = next,
            // A path that is not there answers nil, the same as an absent key would.
            _ => return Own::Nil,
        }
    }
    let key = Value::String(LunaStr::from_slice(&ctx, last.as_bytes()));
    from_lua(ctx, table.get_value(ctx, key))
}

/// Walk `path` from the globals and set the last name, inside the arena.
pub(crate) fn set_field_in<'gc>(ctx: Context<'gc>, path: &[&str], value: &Own) -> bool {
    let Some((last, walked)) = path.split_last() else {
        return false;
    };
    let mut table = ctx.globals();
    for name in walked {
        let key = Value::String(LunaStr::from_slice(&ctx, name.as_bytes()));
        match table.get_value(ctx, key) {
            Value::Table(next) => table = next,
            // A path that is not there is not an error: `package.preload` is absent in a VM built
            // without the library, and the caller's answer to that is to do nothing.
            _ => return false,
        }
    }
    let key = Value::String(LunaStr::from_slice(&ctx, last.as_bytes()));
    table.set(ctx, key, into_lua(ctx, value)).is_ok()
}

/// The host a native sees while the VM is running it.
pub struct CallbackHost<'gc> {
    ctx: Context<'gc>,
}

impl<'gc> CallbackHost<'gc> {
    pub fn new(ctx: Context<'gc>) -> Self {
        CallbackHost { ctx }
    }
}

impl<'gc> Host for CallbackHost<'gc> {
    fn global(&self, name: &str) -> Own {
        let key = Value::String(LunaStr::from_slice(&self.ctx, name.as_bytes()));
        from_lua(self.ctx, self.ctx.globals().get_value(self.ctx, key))
    }

    fn set_global(&self, name: &str, value: Own) {
        let key = Value::String(LunaStr::from_slice(&self.ctx, name.as_bytes()));
        // Fails only on a nil or NaN key, and `name` is a string.
        let _ = self
            .ctx
            .globals()
            .set(self.ctx, key, into_lua(self.ctx, &value));
    }

    fn set_field(&self, path: &[&str], value: Own) -> bool {
        set_field_in(self.ctx, path, &value)
    }

    fn field(&self, path: &[&str]) -> Own {
        field_in(self.ctx, path)
    }

    fn chunk(&self) -> String {
        // The VM stamps positions itself, from the closure's own debug info.
        "?".to_string()
    }

    fn call(&self, function: &Own, args: Vec<Own>) -> LuaResult<Vec<Own>> {
        let ctx = self.ctx;
        let luna::Value::Function(f) = into_lua(ctx, function) else {
            return Err(LuaError::new(
                "attempt to call a value that is not a function",
            ));
        };
        let given: Vec<luna::Value> = args.iter().map(|v| into_lua(ctx, v)).collect();
        run_nested(ctx, luna::Executor::start(ctx, f, luna::Variadic(given)))
    }

    fn eval(&self, source: &str, chunk: &str) -> LuaResult<Vec<Own>> {
        let ctx = self.ctx;
        let closure = luna::Closure::load(ctx, Some(chunk), source.as_bytes())
            .map_err(|e| LuaError::new(e.to_string()))?;
        run_nested(ctx, luna::Executor::start(ctx, closure.into(), ()))
    }

    fn load(&self, source: &str, chunk: &str) -> LuaResult<Own> {
        let ctx = self.ctx;
        let closure = luna::Closure::load(ctx, Some(chunk), source.as_bytes())
            .map_err(|e| LuaError::new(e.to_string()))?;
        Ok(from_lua(ctx, luna::Value::Function(closure.into())))
    }
}

/// Compile a chunk through the callback that is currently running, if one is.
pub fn reentrant_load(source: &str, chunk: &str) -> Option<LuaResult<Own>> {
    with_running(|host| host.load(source, chunk))
}

/// Run a chunk through the callback that is currently running, if one is.
pub fn reentrant_eval(source: &str, chunk: &str) -> Option<LuaResult<Vec<Own>>> {
    with_running(|host| host.eval(source, chunk))
}

thread_local! {
    /// The host of the callback currently running, if one is.
    ///
    /// **So that code far from the VM can still reach it.** A tool a config registered is invoked
    /// by the shell's dispatcher, four Rust frames below the native that started it, and that
    /// dispatcher has no engine to be handed — it calls `call_here`. When the VM is already running
    /// the engine cannot be borrowed, and this is the only route back in.
    static RUNNING: Cell<Option<NonNull<dyn Host>>> = const { Cell::new(None) };
}

/// Run `f` with `host` reachable through [`reentrant`], and restore what was there before.
///
/// The pointer is valid for exactly the body of `f`, which is the callback: it is taken from a
/// borrow that outlives the call, restored on the way out, and restored on an unwind too, because
/// the previous value is put back by a guard. Single-threaded by construction — the slot is
/// thread-local and the engine is `Rc`.
pub(crate) fn while_running<R>(host: &dyn Host, f: impl FnOnce() -> R) -> R {
    /// Puts back whatever was in the slot, however the body leaves.
    struct Restore(Option<NonNull<dyn Host>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            RUNNING.with(|slot| slot.set(self.0));
        }
    }

    // SAFETY: the lifetime is erased only to cross a thread-local. `_guard` puts the previous
    // value back before `host` goes out of scope, on the ordinary path and on an unwind alike, so
    // nothing can observe the pointer after the borrow it came from has ended.
    let erased = unsafe {
        std::mem::transmute::<NonNull<dyn Host + '_>, NonNull<dyn Host + 'static>>(NonNull::from(
            host,
        ))
    };
    let _guard = Restore(RUNNING.with(|slot| slot.replace(Some(erased))));
    f()
}

/// Ask the callback that is currently running, if one is.
///
/// `None` when the VM is idle, which is the case [`Engine`](crate::Engine) handles itself. **Every**
/// entry point that borrows the arena needs this fallback, not just the obvious one: a native that
/// starts a shell command can reach code that reads a global, compiles a `where` expression, or
/// runs a chunk, and each of those would otherwise meet the borrow the callback is holding.
pub fn with_running<R>(f: impl FnOnce(&dyn Host) -> R) -> Option<R> {
    let host = RUNNING.with(|slot| slot.get())?;
    // SAFETY: the slot holds a pointer only while `while_running` is on the stack below us, and
    // that function does not return until the borrow it made is finished with.
    Some(f(unsafe { host.as_ref() }))
}

/// Call a Lua function through the callback that is currently running, if one is.
pub fn reentrant(function: &Own, args: Vec<Own>) -> Option<LuaResult<Vec<Own>>> {
    with_running(|host| host.call(function, args))
}

/// Drive `executor` to the end from inside a running callback.
///
/// **This is what makes the shell's re-entrancy work on a stackless VM.** A script calls
/// `oslo.proc.exec("greet")`; that native runs the shell; the shell dispatches a tool a config
/// registered; the tool's body is a Lua function. Three of those four frames are Rust, so there is
/// no way to express the round trip as a continuation — the call has to happen *here*, part way
/// down a native.
///
/// It works because stepping needs only a [`Context`], which a callback already holds — the
/// `&mut Lua` that [`Engine`](crate::Engine) borrows is needed for the collector, not for running
/// code. So a nested call runs on its own executor in the same arena, and nothing is collected
/// until the outermost call returns. That is the cost: a deeply nested chain holds its garbage.
fn run_nested<'gc>(ctx: Context<'gc>, executor: luna::Executor<'gc>) -> LuaResult<Vec<Own>> {
    // Enough that an ordinary handler finishes in one step, and small enough that a runaway loop
    // is still interruptible by the fuel accounting rather than by the stack.
    const FUEL_PER_STEP: i32 = 4096;
    loop {
        let mut fuel = luna::Fuel::with(FUEL_PER_STEP);
        match executor.step(ctx, &mut fuel) {
            Ok(true) => break,
            Ok(false) => continue,
            Err(e) => return Err(LuaError::new(e.to_string())),
        }
    }
    match executor.take_result::<luna::Variadic<Vec<luna::Value>>>(ctx) {
        Ok(Ok(luna::Variadic(values))) => {
            Ok(values.into_iter().map(|v| from_lua(ctx, v)).collect())
        }
        Ok(Err(error)) => Err(LuaError::without_position(error.to_string())),
        Err(e) => Err(LuaError::new(e.to_string())),
    }
}

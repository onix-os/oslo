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

    /// What the running source is called, for the position on an error.
    fn chunk(&self) -> String;

    /// Call a Lua function.
    ///
    /// Refused while a native is running — see the module note on stacklessness.
    fn call(&self, function: &Own, args: Vec<Own>) -> LuaResult<Vec<Own>>;
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

    fn chunk(&self) -> String {
        // The VM stamps positions itself, from the closure's own debug info.
        "?".to_string()
    }

    fn call(&self, _function: &Own, _args: Vec<Own>) -> LuaResult<Vec<Own>> {
        Err(LuaError::new(
            "cannot call a Lua function from inside a native call: the VM is stackless, so this \
             binding must be written as a sequence",
        ))
    }
}

//! The Lua VM, behind the same front door as the evaluator it replaces.
//!
//! `luna` is a stackless bytecode VM with a tracing collector, in pure Rust. Against the tree walker
//! in `oslo-lua` that is coroutines, `goto`, real string patterns, byte-exact strings, collected
//! cycles and unbounded recursion — measured at 15–30× the speed — with no C anywhere, so a static
//! musl build still needs nothing installed.
//!
//! # The one thing that shapes everything above this
//!
//! luna's values carry a garbage-collector lifetime: `Value<'gc>` exists only inside
//! `lua.enter(|ctx| …)`, and holding one across calls means stashing it in the VM's registry. That
//! is the opposite of `oslo_lua::Value`, which is an `Rc` anyone can keep. So the boundary is real
//! and it lives here: oslo's own value type stays the interchange currency for the ~40 files that
//! have nothing to do with Lua — the structured pipeline, settings, hooks — and is converted at the
//! edge, once, rather than infecting them with a lifetime.

use luna::{Closure, Executor, Lua};

/// Run `source` and answer the exit status, reporting an error the way the shell would.
pub fn run(source: &str, chunk_name: &str) -> i32 {
    let mut lua = Lua::full();
    let executor = match lua.try_enter(|ctx| {
        let closure = Closure::load(ctx, Some(chunk_name), source.as_bytes())?;
        Ok(ctx.stash(Executor::start(ctx, closure.into(), ())))
    }) {
        Ok(executor) => executor,
        Err(error) => {
            eprintln!("oslo: lua: {error}");
            return 1;
        }
    };
    match lua.execute::<()>(&executor) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("oslo: lua: {error}");
            1
        }
    }
}

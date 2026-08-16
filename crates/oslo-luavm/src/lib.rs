//! A measurable spike: the reference Lua 5.4 VM, in the same process as the shell.
//!
//! `oslo-lua` is a tree walker over `full_moon`'s AST, and `docs/features/lua-interpreter.md`
//! records what that costs — no coroutines, no tracing GC, no byte strings, 200 nested calls — and
//! what it buys: one Rust core behind two front ends, and no C anywhere near a static musl build.
//!
//! This crate exists to put a number on the other side of that trade rather than an opinion. It is
//! deliberately *not* wired into the shell's `oslo.*` API — that surface is 130 callables across
//! 11.5k lines, and porting it is the actual cost of the switch, not something to fake in a spike.
//! What it does is run the same benchmark scripts the documented measurements use, through the real
//! VM, inside a real oslo binary, so the two engines can be compared on identical work.

use mlua::Lua;

/// Run `source` and answer the exit status, printing any error the way the shell would.
pub fn run(source: &str, chunk_name: &str) -> i32 {
    let lua = Lua::new();
    match lua.load(source).set_name(chunk_name).exec() {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("oslo: lua: {error}");
            1
        }
    }
}

/// What this build speaks, for a caller that wants to say so.
pub fn version() -> &'static str {
    mlua::Lua::new()
        .load("return _VERSION")
        .eval::<String>()
        .map(|_| "Lua 5.4 (mlua, vendored)")
        .unwrap_or("Lua 5.4 (mlua)")
}

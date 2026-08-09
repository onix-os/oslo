pub mod api;
pub mod columns;
pub mod context;
pub mod engine;
pub mod parsed;

/// The interpreter, which is its own crate.
///
/// It depends on nothing but its parser — no shell, no terminal, no environment — which is what
/// makes it worth publishing separately and what puts it at the bottom of this one's graph. Kept
/// reachable under the name it had so that `lua::eval::…` still reads the same everywhere it is
/// used; the alternative was a thousand-line rename that said nothing.
pub use oslo_lua as eval;

pub use engine::LuaEngine;

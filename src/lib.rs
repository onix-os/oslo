/// The stack oslo runs its own work on, rather than whatever `ulimit -s` happens to be.
///
/// oslo's Lua is a tree-walker: a Lua function calling a Lua function is a chain of Rust frames,
/// so Lua's recursion depth is bounded by the Rust stack. Real Lua is not — it keeps Lua-to-Lua
/// calls on its own heap-allocated stack — so the only way to give a script a predictable limit is
/// to stop depending on the ambient one. A shell inherits its stack from whoever spawned it, which
/// under a service manager can be as little as 512 KiB.
///
/// The reservation is virtual: pages are committed as they are touched, so an `oslo` that never
/// runs Lua pays nothing for this. `crate::lua::eval` refuses at a depth chosen to fit well inside
/// it, and `lua_eval_tests` runs its depth cases on a thread of exactly this size so the limit is
/// checked against the stack oslo actually provides.
pub const INTERPRETER_STACK: usize = 16 * 1024 * 1024;

/// The bottom of the stack, which is its own crate.
///
/// The syntax tree, the error type, the feature bits, the hook registry and the tracking store —
/// five things that do not know there is a shell above them. Kept reachable under the names they
/// had, so `crate::ast::…` and `crate::error::…` still read the same in the thousand places that
/// use them; the alternative was a rename that said nothing.
pub use oslo_base::{ast, error, feature, hooks, nesting, track};

pub mod data;
pub mod direnv;
pub mod env;
pub mod exec;
pub mod expand;
pub mod lexer;
pub mod lua;
pub mod parser;
/// SSH, behind the `ssh` feature — off by default. See the module docs for what it costs and what
/// is still undecided.
#[cfg(feature = "ssh")]
pub mod ssh;
pub mod ui;

pub use env::Environment;
pub use error::{Result, ShellError};
pub use exec::{JobManager, eval_command_list};
pub use lexer::Lexer;
pub use lua::LuaEngine;
pub use parser::parse_bash_script;
pub use ui::OsloHelper;

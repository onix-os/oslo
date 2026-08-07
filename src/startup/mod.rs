//! What the shell does around the user's program: startup files, prompts, history, Lua.
//!
//! These live in the *binary* rather than the library on purpose. Every one of them reads the
//! real user's `$HOME`, sources arbitrary files, or holds process-global state; a library that
//! did any of that behind a caller's back would make the shell impossible to test in-process,
//! which is exactly the trap `ui/history_expand.rs` is kept out of too.
//!
//! Split by the question each file answers:
//!
//! * [`rc`] — which files a new shell reads before the first command, and what the prompt says.
//! * [`history`] — where the history lives, how big it is, and the `history` builtin.
//! * [`lua_init`] — the optional `init.lua` layer, and what happens when it is broken.
//! * [`repl`] — the interactive loop that uses all three.
//! * `tracking` — what that loop hands [`oslo::track`] instead of discarding.

pub mod config;
mod environments;
pub mod history;
mod integration;
pub mod language;
pub mod lua_init;
pub mod mode;
pub mod native;
pub mod prompt;
pub mod rc;
mod read;
pub mod recall;
pub mod repl;
mod report;
mod tracking;

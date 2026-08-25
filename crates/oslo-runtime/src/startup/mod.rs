//! What the shell does around the user's program: startup files, prompts, history, Lua.
//!
//! These sit at the *top* of the stack, above everything they drive. Every one of them reads the
//! real user's `$HOME`, sources arbitrary files, or holds process-global state — so nothing below
//! may reach them, and nothing here is on a path a library caller can take by accident. That is
//! the same rule `history_expand` follows by living in the binary alone: it rewrites a line before
//! it is parsed, so a `-c` or a script reaching it would let data turn into a different command.
//!
//! Split by the question each file answers:
//!
//! * [`rc`] — which files a new shell reads before the first command, and what the prompt says.
//! * [`history`] — where the history lives, how big it is, and the `history` builtin.
//! * [`lua_init`] — the optional `init.lua` layer, and what happens when it is broken.
//! * [`repl`] — the interactive loop that uses all three.
//! * `tracking` — what that loop hands [`oslo_base::track`] instead of discarding.

mod arrival;
pub mod config;
#[cfg(feature = "direnv")]
mod environments;
mod follow;
pub mod history;
mod integration;
pub mod language;
pub mod lua_init;
pub mod mode;
pub mod native;
mod nested;
mod plugins;
pub mod prompt;
pub mod rc;
mod read;
pub mod recall;
pub mod repl;
#[cfg(feature = "direnv")]
mod report;
mod servicing;
mod stored;
mod terminal;
mod timers;
pub mod timing;
mod tracking;
mod transcript;

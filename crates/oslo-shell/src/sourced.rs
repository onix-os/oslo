//! Sourcing a file that is not shell.
//!
//! # The language is detected here too, or it is not detected at all
//!
//! `oslo script.lua` works because [`run_script`](../../src/main.rs) asks `language::detect` what it
//! is holding. `source script.lua` did not ask, so a Lua file went to the shell parser and came back
//! as `syntax error at line 2 col 20` — in a shell whose own rule is that **Lua never needs an
//! opt-in flag**. One of the two entry points was applying that rule.
//!
//! # Why it is a slot rather than a call
//!
//! Detection and the interpreter both live in `oslo-runtime`, which is *above* this crate — the same
//! direction [`crate::data::custom`] documents for registered tools, and for the same reason. So the
//! shell asks a function somebody put here, and knows nothing about what is behind it. With nothing
//! installed the answer is `None` and `source` behaves exactly as it always did.
//!
//! # What a sourced Lua file can and cannot do
//!
//! It can **register** things: `oslo.register_tool` puts a handler in a thread-local table, so a
//! script that sources its own tools can then use them — which is the whole point, and the gap that
//! made a config-registered verb work at the prompt and vanish in a script.
//!
//! It cannot set shell variables. That is not a shortcut taken here: Lua reached from inside a
//! builtin runs *while the shell holds its own state*, so every route back to the `Environment`
//! raises or answers nil. [`crate::env::view`] is the codebase's standing answer to that, and this
//! inherits it rather than inventing a second one.

use std::sync::OnceLock;

/// Answers the status, or `None` when the file is not this runner's language and `source` should
/// parse it as shell.
type Runner = fn(&str, &str) -> Option<i32>;

static RUNNER: OnceLock<Runner> = OnceLock::new();

/// Install the runner. The first one wins; a second call is ignored.
pub fn install(runner: Runner) {
    let _ = RUNNER.set(runner);
}

/// Run `text` if it is not shell. `None` means it is, or that nothing was installed.
pub fn run_if_not_shell(path: &str, text: &str) -> Option<i32> {
    RUNNER.get().and_then(|run| run(path, text))
}

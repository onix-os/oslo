//! The pieces of the loop that are not the loop: what it says to the terminal, and what it makes
//! of a Lua line.
//!
//! Split out when `repl.rs` crossed the 600-line limit. Grouped by subject rather than by the
//! order the loop happens to call them in — the terminal-facing helpers together, the Lua-line
//! evaluator with them because it is the other thing the loop delegates whole rather than inlines.

use oslo::Environment;
use oslo::LuaEngine;
use oslo::error::ShellError;
use std::sync::{Arc, Mutex};

pub(crate) fn announce(sequence: &str) {
    if sequence.is_empty() {
        return;
    }
    print!("{sequence}");
    let _ = std::io::Write::flush(&mut std::io::stdout());
}

/// The title to show while `text` runs: the command, shortened to its first word and trimmed.
///
/// The first word rather than the whole line, because a title bar is narrow and `cargo` in a tab
/// is more use than the first forty characters of a `for` loop.
pub(crate) fn title_for_command(text: &str) -> String {
    let first = text.split_whitespace().next().unwrap_or("");
    if first.is_empty() {
        current_directory()
    } else {
        format!(
            "{first} — {}",
            oslo::ui::prompt::tilde(&current_directory())
        )
    }
}

/// Where the shell is now, for the `cd` hook to compare against.
pub(crate) fn cwd() -> String {
    current_directory()
}

pub(crate) fn current_directory() -> String {
    std::env::current_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Run one Lua line typed at the prompt.
///
/// A chunk that merely printed something has not run a command, so `$?` stays where it was;
/// `oslo.proc.exit(n)` is how a script chooses a status, and it ends the shell rather than setting one.
pub(crate) fn run_lua_line(
    lua: &LuaEngine,
    text: &str,
    last_status: i32,
) -> Result<i32, ShellError> {
    match lua.eval_script(text) {
        Ok(()) => Ok(last_status),
        Err(ShellError::Lua(e)) if e.exit.is_some() => Err(ShellError::Exit(e.exit.unwrap_or(0))),
        Err(e) => Err(e),
    }
}

/// `$IGNOREEOF`: how many end-of-file characters to ignore before ending the shell.
///
/// `None` means the variable is unset and Ctrl-D exits immediately, as it always has. bash's
/// documented fallback for a value that is not a number is 10.
pub(crate) fn ignore_eof_limit(env_struct: &Arc<Mutex<Environment>>) -> Option<usize> {
    let guard = env_struct.lock().unwrap();
    if let Some(raw) = guard.get_var("IGNOREEOF") {
        return Some(raw.trim().parse::<usize>().unwrap_or(10));
    }
    // `set -o ignoreeof` is the option spelling of the same thing, and bash treats it as
    // `IGNOREEOF=10`. It used to be accepted and ignored, so a shell that had been told not to
    // exit on Ctrl-D exited on Ctrl-D.
    guard
        .option(oslo::env::options::ShellOption::IgnoreEof)
        .then_some(10)
}

thread_local! {
    /// How long the command before this prompt took, for the right prompt to mention.
    ///
    /// Thread-local rather than threaded through `read_command`: the duration is a property of the
    /// session's last command, and every caller that wants it is on the REPL's own thread.
    static LAST_DURATION: std::cell::Cell<Option<std::time::Duration>> =
        const { std::cell::Cell::new(None) };
}

/// How long the last command took, if one has run.
pub(crate) fn last_command_duration() -> Option<std::time::Duration> {
    LAST_DURATION.get()
}

pub(crate) fn note_command_duration(elapsed: std::time::Duration) {
    LAST_DURATION.set(Some(elapsed));
}

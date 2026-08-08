//! Running the shell code a bash integration installed.
//!
//! One hook now: `$PROMPT_COMMAND`, run from the read loop before every prompt. It lives here
//! rather than in `repl` because its failures are somebody else's code failing and must never
//! take the shell with them, which is a different contract from the rest of the loop.
//!
//! There used to be a second: `bind -x`, which gave a shell command a keystroke and handed it the
//! line in `$READLINE_LINE`. It was removed — about 875 lines for a feature only interactive
//! plugins use, in a shell whose first job is to be `/bin/sh`. `oslo.keys` remains for binding a
//! key to a Lua function.

use oslo_shell::Environment;
use oslo_shell::exec::eval_command_list;
use oslo_shell::syntax::parse_with_aliases;
use std::sync::{Arc, Mutex};

/// Run `$PROMPT_COMMAND`, bash's "before every prompt" hook.
///
/// The counterpart to the DEBUG trap: DEBUG fires before a command, this fires before the prompt
/// that follows it, and between them they are what every bash integration hangs off. hexe sets
/// `PROMPT_COMMAND="__shp_precmd;__hexe_precmd"` — one rebuilds `PS1`, the other reports the
/// command that just ended — and without this it installs perfectly and then does nothing at all.
///
/// Two details are load-bearing, both because a hook is written expecting them:
///
/// * **`$?` is the finished command's status, and survives.** `__shp_precmd` opens with
///   `local exit_status=$?`, so a hook that ran with `$?` already clobbered would colour every
///   prompt as a success. It is restored afterwards too, or the hook's own last command would
///   become the status the *next* prompt reports;
/// * **an error is reported and dropped.** A broken `PROMPT_COMMAND` must not take the shell with
///   it. bash prints the diagnostic and carries on, and a prompt hook is exactly the code most
///   likely to be half-written.
///
/// bash 5.1's array form — several `PROMPT_COMMAND` elements run in turn — is not supported; the
/// scalar is what integrations emit, and oslo has no associative-array machinery behind it yet.
pub(super) fn prompt_command(env_struct: &Arc<Mutex<Environment>>, last_status: i32) {
    let mut env = match env_struct.lock() {
        Ok(env) => env,
        Err(poisoned) => poisoned.into_inner(),
    };
    let Some(text) = env.get_param("PROMPT_COMMAND") else {
        return;
    };
    if text.trim().is_empty() {
        return;
    }

    env.last_status = last_status;
    let outcome = parse_with_aliases(&text, &|name| env.get_alias(name).map(str::to_string))
        .and_then(|ast| eval_command_list(&mut env, &ast));
    if let Err(e) = outcome {
        eprintln!("oslo: PROMPT_COMMAND: {e}");
    }
    env.last_status = last_status;
}

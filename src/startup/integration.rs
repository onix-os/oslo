//! Running the shell code a bash integration installed.
//!
//! Two hooks, both of which run *from the read loop* rather than from inside the editor, and both
//! of which exist because the ecosystem is written against bash:
//!
//! * [`run`] — a `bind -x` command, with the line handed to it in `$READLINE_LINE`;
//! * [`prompt_command`] — `$PROMPT_COMMAND`, before every prompt.
//!
//! They are in one file because they are one concern: shell code the user did not type, run by
//! the loop, whose failures must never take the shell down.
//!
//! # `bind -x`
//!
//! The editor cannot do this itself. Its handler runs inside rustyline's own read loop with the
//! terminal in raw mode, and a bound command is arbitrary shell code that may want to draw a
//! full-screen picker — atuin's Ctrl-R does exactly that. So the handler only *records* the
//! request and ends the line; this runs afterwards, from the read loop, where the terminal has
//! been restored and the shell is free to do anything.
//!
//! # The exchange
//!
//! ```text
//!   key pressed  ->  $READLINE_LINE  = the buffer
//!                    $READLINE_POINT = the cursor, in bytes
//!                    <command runs>
//!                 <- $READLINE_LINE  = what the prompt comes back with
//!                    $READLINE_POINT = where the cursor goes
//! ```
//!
//! Both variables are read back **after** the command, because that is the whole mechanism a
//! plugin replaces your line with: atuin's picker writes the chosen command into `READLINE_LINE`
//! and returns. A command that leaves them alone leaves the line alone.
//!
//! The line is never submitted here. Control goes back to the prompt with whatever the command
//! left, exactly as under bash — a plugin that wants the line to run sets it and the user presses
//! Enter.

use oslo::Environment;
use oslo::exec::eval_command_list;
use oslo::interactive::readline::Request;
use oslo::parser::parse_with_aliases;
use std::sync::{Arc, Mutex};

/// Where the prompt comes back, once a bound command has had the line.
pub(super) struct Outcome {
    pub line: String,
    /// Byte offset for the cursor, clamped to the line by [`run`] so the caller cannot be handed
    /// a position outside it.
    pub point: usize,
}

/// Run one `bind -x` command and report what the line should become.
///
/// Errors are reported and then dropped: a broken keybinding must give the prompt back, not take
/// the shell down with it. That matters more here than in most places, because the code being run
/// came from a plugin the user only `eval`'d.
pub(super) fn run(env_struct: &Arc<Mutex<Environment>>, request: &Request) -> Outcome {
    let mut env = match env_struct.lock() {
        Ok(env) => env,
        Err(poisoned) => poisoned.into_inner(),
    };

    // The cursor is a byte offset, as bash's `$READLINE_POINT` is, and the buffer may hold
    // multibyte text — so it is clamped to a character boundary before anyone can slice with it.
    let point = boundary(&request.line, request.point);
    env.set_var("READLINE_LINE", &request.line, false);
    env.set_var("READLINE_POINT", &point.to_string(), false);

    let outcome = oslo::parser::parse_with_aliases(&request.command, &|name| {
        env.get_alias(name).map(str::to_string)
    })
    .and_then(|ast| eval_command_list(&mut env, &ast));
    if let Err(e) = outcome {
        eprintln!("oslo: bind: {}: {e}", request.command);
    }

    let line = env.get_param("READLINE_LINE").unwrap_or_default();
    let point = env
        .get_param("READLINE_POINT")
        .and_then(|text| text.trim().parse::<usize>().ok())
        .unwrap_or(line.len());
    let point = boundary(&line, point);

    // Unset afterwards, as bash does: they describe one keystroke, and a command run later that
    // found them still set would be reading a line nobody is editing.
    env.unset_var("READLINE_LINE");
    env.unset_var("READLINE_POINT");

    Outcome { line, point }
}

/// The nearest character boundary at or below `at`, so slicing the line can never panic.
fn boundary(line: &str, at: usize) -> usize {
    let mut at = at.min(line.len());
    while at > 0 && !line.is_char_boundary(at) {
        at -= 1;
    }
    at
}

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

#[cfg(test)]
mod tests {
    use super::boundary;

    /// A cursor offset arriving from shell code is a number a user could have written, so it has
    /// to be treated as untrusted: past the end, or inside a multibyte character, must both be
    /// safe rather than a panic in the middle of the prompt.
    #[test]
    fn a_cursor_offset_is_clamped_to_the_line() {
        assert_eq!(boundary("hello", 3), 3);
        assert_eq!(boundary("hello", 99), 5);
        assert_eq!(boundary("", 4), 0);
        // `é` is two bytes: offset 1 is inside it and moves back to 0.
        assert_eq!(boundary("é", 1), 0);
        assert_eq!(boundary("aé", 2), 1);
        assert_eq!(boundary("aé", 3), 3);
    }
}

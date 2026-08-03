//! Running a `bind -x` command, with the line handed to it and taken back.
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

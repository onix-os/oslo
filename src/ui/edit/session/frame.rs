//! Turning the session's state into the bytes a terminal draws, and reading the key that changes
//! it next.
//!
//! Split from the loop when `session.rs` crossed the 600-line limit. These are the four things the
//! loop delegates whole rather than inlines: waiting for a key, laying out a frame, converting a
//! layout into a cursor position, and the no-terminal path that does none of it.

use super::super::{layout, screen};
use super::{Assist, Outcome, Session};
use crate::ui::dropdown::terminal_cols;
use crate::ui::term::{Key, Keys};

/// The next key, firing `on-idle-timeout` if the prompt sits untouched long enough.
///
/// **The blocking read is the default and stays the default.** A timed read is only asked for when
/// `oslo.misc.idle_timeout` is set *and* something is attached to the hook — otherwise the editor
/// would wake up on a timer for the rest of the session to ask a question nobody is listening for.
///
/// `reported` is what stops it firing over and over: idleness is a state you enter once, not a
/// tick. It resets the moment a key arrives, so walking away twice reports twice.
pub(super) fn next_key(keys: &mut Keys, reported: &mut bool) -> Option<Key> {
    let seconds = crate::ui::settings::current().misc.idle_timeout;
    if seconds == 0 || !crate::lua::api::hooks::watched(crate::lua::api::hooks::at::IDLE_TIMEOUT) {
        return keys.read();
    }
    let ms = seconds.saturating_mul(1000).min(i32::MAX as u64) as i32;
    loop {
        match keys.read_within(ms) {
            crate::ui::term::Pressed::Key(key) => {
                *reported = false;
                return Some(key);
            }
            crate::ui::term::Pressed::Timeout => {
                if !*reported {
                    *reported = true;
                    crate::lua::engine::fire_at_here(
                        crate::lua::api::hooks::at::IDLE_TIMEOUT,
                        &[("seconds", &seconds.to_string())],
                    );
                }
            }
            crate::ui::term::Pressed::Ended => return None,
        }
    }
}

/// Build the frame for the current state.
/// Lay the line out. `ghost` is whether the suggestion is drawn with it.
///
/// **Off for the last frame of a line.** The ghost is a proposal, not text you typed — so once the
/// line is finished it has to go, or the transcript shows a command that was never run. Typing
/// `cat ~/` with `lis/` suggested and pressing Enter left `cat ~/lis/` on screen above the output
/// of `cat ~/`, which is a scrollback that lies about what happened.
pub(super) fn draw(
    prompt: &str,
    right: &str,
    session: &Session,
    assist: &mut dyn Assist,
    ghost: bool,
) -> layout::Placed {
    let plain = session.buffer.text();
    let painted = assist.highlight(&plain);
    let hint = if ghost {
        assist
            .hint(&plain, session.buffer.cursor())
            .unwrap_or_default()
    } else {
        String::new()
    };
    layout::place(&layout::Row {
        prompt,
        text: &painted,
        plain: &plain,
        cursor: session.buffer.cursor(),
        hint: &hint,
        right,
        // Read every frame rather than cached, so a resized terminal lays out correctly on the
        // next keystroke without a `SIGWINCH` handler to get wrong.
        cols: terminal_cols(),
    })
}

pub(super) fn into_at(placed: &layout::Placed) -> screen::At {
    screen::At {
        rows: placed.rows,
        cursor_row: placed.cursor_row,
        cursor_col: placed.cursor_col,
    }
}

/// A line from stdin, for when there is no terminal to edit on.
pub(super) fn read_plain(prompt: &str) -> Outcome {
    // The prompt is written only for a terminal that cannot be edited on — `TERM=dumb`, a serial
    // console, a `screen` session someone has told to be simple. Down an ordinary pipe it is not
    // written at all: the shell is being driven by a script, and a prompt interleaved with the
    // output would be noise in the middle of the data.
    if std::env::var("TERM").as_deref() == Ok("dumb") {
        print!("{prompt}");
        let _ = std::io::Write::flush(&mut std::io::stdout());
    }
    let mut line = String::new();
    match std::io::BufRead::read_line(&mut std::io::stdin().lock(), &mut line) {
        Ok(0) | Err(_) => Outcome::Eof,
        Ok(_) => {
            while line.ends_with('\n') || line.ends_with('\r') {
                line.pop();
            }
            Outcome::Line(line)
        }
    }
}

/// The first word of a suggestion, with the whitespace that follows it.
///
/// Accepting "one word" of `--example foo` should leave the cursor after the space, ready for the
/// next word — stopping before it would make the second press insert a leading space.
pub(super) fn first_word(hint: &str) -> String {
    let trimmed = hint.trim_start();
    let lead = hint.len() - trimmed.len();
    let end = trimmed
        .find(char::is_whitespace)
        .map(|at| {
            let rest = &trimmed[at..];
            at + rest.len() - rest.trim_start().len()
        })
        .unwrap_or(trimmed.len());
    hint[..lead + end].to_string()
}

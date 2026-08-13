//! Asking before starting a shell inside a shell.
//!
//! Typing `oslo` at an oslo prompt has always worked and has never said anything, which is the
//! problem: the new shell looks exactly like the old one, so the only sign you are two deep is an
//! `exit` later that does not close the terminal. Every accidental nesting is discovered that way.
//!
//! So an interactive oslo that finds an oslo above it asks first, and the answer that costs nothing
//! is the one Enter gives you: stay where you are. [`oslo_base::track::nested`] keeps the count and
//! publishes it as `$OSLO_NESTED`, for a prompt that wants to show it.
//!
//! # When it does not ask
//!
//! * **Nothing above it.** A fresh terminal is not nesting.
//! * **No terminal to ask on.** `command | oslo -i` is a shell with no person at the other end of
//!   stdin; a question there would be answered by the script's first line. This is why the check is
//!   for a terminal rather than for `interactive`, which `-i` sets regardless.
//! * **`oslo.misc.nested_ask = false`**, for somebody who nests on purpose.
//!
//! A tool never asks either, because a tool never gets here: `oslo macros` is not a shell and takes
//! no place in the count.

use oslo_ui::ask::{self, Answer, Confirm};

/// Ask, and leave rather than nest if the answer is no.
///
/// Called after the config has been read — so `oslo.misc.nested_ask` is in force — and before the
/// expensive part of starting a shell, so a "no" pays for none of it.
pub fn ask_before_nesting() {
    let depth = oslo_base::track::nested::depth();
    if depth == 0 || !oslo_ui::settings::current().misc.nested_ask || !someone_is_there() {
        return;
    }

    let answer = ask::confirm(&Confirm {
        question: format!(
            "You are already in oslo. Start a nested shell? ({} deep)",
            ordinal(depth)
        ),
        yes: "Nested shell".to_string(),
        no: "Stay here".to_string(),
        // Enter does the thing you can undo. Nesting by mistake is the case this exists for.
        default: false,
        chrome: Default::default(),
    });

    if !matches!(answer, Answer::Given(true)) {
        // Not a failure: leaving a shell you did not mean to start is the point of asking, and a
        // non-zero status would make `oslo && …` in somebody's script read it as one.
        std::process::exit(0);
    }
}

/// Whether there is a person at the other end to answer.
///
/// Both ends, as `stdin_is_a_terminal` in the binary checks: a question drawn on a redirected
/// stderr is a question nobody sees, answered by whatever stdin holds next.
fn someone_is_there() -> bool {
    nix::unistd::isatty(0).unwrap_or(false) && nix::unistd::isatty(2).unwrap_or(false)
}

/// `1` → "one", for a sentence rather than a number in the middle of one.
fn ordinal(depth: usize) -> String {
    match depth {
        1 => "one".to_string(),
        2 => "two".to_string(),
        3 => "three".to_string(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The small words are worth spelling; past three, a number reads better than "seventeen".
    #[test]
    fn the_depth_is_said_as_a_sentence_would_say_it() {
        assert_eq!(ordinal(1), "one");
        assert_eq!(ordinal(3), "three");
        assert_eq!(ordinal(17), "17");
    }
}

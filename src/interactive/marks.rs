//! Semantic marks: telling the terminal where a command begins and ends.
//!
//! `OSC 133`, the FinalTerm/FTCS shell-integration protocol that kitty, WezTerm, Ghostty, iTerm2,
//! VS Code and tmux all read. The shell says where the prompt starts, where output starts, and
//! what the command exited with; what the terminal *does* with that is the terminal's business.
//!
//! # Why the shell only marks, and does not fold
//!
//! A shell cannot rewrite scrollback. Once bytes are written they belong to whatever owns the
//! grid, and folding a command that has scrolled off means redrawing rows the shell can no longer
//! reach. The thing that *can* do it is the terminal emulator or the multiplexer, which keeps the
//! grid and the history. So the division is: oslo declares the boundaries, and the layer that owns
//! the screen decides whether to draw a fold arrow next to them.
//!
//! # What is emitted
//!
//! | Mark | When | Meaning |
//! |---|---|---|
//! | `OSC 133 ; A ; aid=<n> ST` | before the prompt is drawn | prompt start; `aid` is the block id |
//! | `OSC 133 ; C ; aid=<n> ST` | just before the command runs | output starts here |
//! | `OSC 133 ; D ; <status> ; aid=<n> ST` | once it has finished | command end, with its exit status |
//!
//! `B` — "the prompt ends and typing starts" — is deliberately **not** emitted. It would have to
//! be written between the prompt and the cursor, which means inside the string handed to the line
//! editor, and the editor measures that string to work out where the line begins. An `OSC` in
//! there is counted as visible width and the cursor arithmetic is wrong from the first keystroke.
//! `A`..`C` already delimits the prompt, which is what a folding implementation needs.
//!
//! `aid` is oslo's addition to the standard three: it makes each block nameable, so a reader can
//! match a `D` to the `A` that opened it without relying on them being adjacent in the stream.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

static ENABLED: AtomicBool = AtomicBool::new(false);
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// Turn marks on for an interactive session that has a terminal to mark.
///
/// Off for every script, `-c`, and test binary: a program reading oslo's output must never find
/// escape sequences the shell invented in it.
pub fn enable(interactive: bool) {
    let on = interactive
        && nix::unistd::isatty(1).unwrap_or(false)
        && std::env::var("TERM").map(|t| t != "dumb").unwrap_or(true);
    ENABLED.store(on, Ordering::Relaxed);
}

pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// The id of the block being prompted for.
pub fn current_id() -> u64 {
    NEXT_ID.load(Ordering::Relaxed)
}

/// Take an id and move to the next. Called once per prompt.
fn advance() -> u64 {
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

/// `OSC 133 ; A` — a new prompt, and a new block, begins here.
pub fn prompt_start() -> String {
    if !enabled() {
        return String::new();
    }
    format!("\x1b]133;A;aid={}\x1b\\", advance())
}

/// `OSC 133 ; C` — everything after this is the command's output.
pub fn output_start() -> String {
    if !enabled() {
        return String::new();
    }
    // `current_id` and not a fresh one: this closes the prompt `A` opened, so it carries the same
    // id. `A` has already advanced the counter, so the id in force is the one before it.
    format!("\x1b]133;C;aid={}\x1b\\", current_id().saturating_sub(1))
}

/// `OSC 133 ; D` — the command has finished, with this status.
pub fn command_end(status: i32) -> String {
    if !enabled() {
        return String::new();
    }
    format!(
        "\x1b]133;D;{status};aid={}\x1b\\",
        current_id().saturating_sub(1)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Marks are off unless a person is looking at a terminal. A script's output must never carry
    /// escape sequences the shell invented.
    #[test]
    fn nothing_is_emitted_without_a_terminal() {
        enable(false);
        assert_eq!(prompt_start(), "");
        assert_eq!(output_start(), "");
        assert_eq!(command_end(0), "");
    }

    /// A block's three marks carry the same id, so a reader can pair them up without assuming
    /// they arrive next to each other.
    #[test]
    fn one_block_carries_one_id_through_all_three_marks() {
        ENABLED.store(true, Ordering::Relaxed);
        NEXT_ID.store(7, Ordering::Relaxed);

        let a = prompt_start();
        let c = output_start();
        let d = command_end(3);
        assert_eq!(a, "\x1b]133;A;aid=7\x1b\\");
        assert_eq!(c, "\x1b]133;C;aid=7\x1b\\");
        assert_eq!(d, "\x1b]133;D;3;aid=7\x1b\\");

        // The next prompt is the next block.
        assert_eq!(prompt_start(), "\x1b]133;A;aid=8\x1b\\");
        assert_eq!(output_start(), "\x1b]133;C;aid=8\x1b\\");

        ENABLED.store(false, Ordering::Relaxed);
    }

    /// Every mark is a complete OSC: introducer, payload, terminator. A half-written one would be
    /// swallowed along with whatever text followed it.
    #[test]
    fn every_mark_is_a_terminated_osc() {
        ENABLED.store(true, Ordering::Relaxed);
        for mark in [prompt_start(), output_start(), command_end(0)] {
            assert!(mark.starts_with("\x1b]133;"), "{mark:?}");
            assert!(mark.ends_with("\x1b\\"), "{mark:?}");
            assert!(
                !mark.contains('\n'),
                "a mark must not move the cursor: {mark:?}"
            );
        }
        ENABLED.store(false, Ordering::Relaxed);
    }
}

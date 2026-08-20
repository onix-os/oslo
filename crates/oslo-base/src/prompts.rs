//! The command lines of previous prompts, newest first.
//!
//! **Lines, not output**, and the difference is worth being plain about. A pipeline stage's output
//! can be captured for nothing, because a stage already writes to a pipe. A *command's* output goes
//! to the terminal, and standing between the two turns `isatty` false for everything — see
//! [`crate::capture`], where that argument is made at length. What a previous prompt does have,
//! free and exactly, is the line that was typed:
//!
//! ```text
//! $ cat one.txt two.txt
//! $ wc -l {-1:0:1}          → wc -l one.txt
//!         └─ previous prompt, its only line, word 1
//! ```
//!
//! So `{-n:…}` addresses the command *line* — one line, its words being the command and its
//! arguments. `{-1:0:-1}` is the last argument, which is `!$` written in [`crate::coords`]'
//! grammar and usable where `!$` is not: inside a script, inside a function, inside quotes.
//!
//! # Why it lives here rather than beside the substitution
//!
//! It was in `oslo_shell::exec::streams`, next to the code that reads it, which is the obvious
//! place. But the *line editor* wants it too: a coordinate reaching back into the session is the
//! one kind that can be resolved **before Enter**, and showing what `{-1:0:1}` will become is worth
//! more than showing it afterwards. `oslo-ui` deliberately does not depend on `oslo-shell` — the
//! editor draws a line, it does not run one — and this is a `Vec<String>` with three operations,
//! so it moved down to the crate they already share rather than a dependency being added for it.

use std::cell::RefCell;

/// How many prompts back a coordinate may reach.
///
/// Ten is the number of things anybody keeps in their head; past that you look at the screen.
pub const KEPT: usize = 10;

thread_local! {
    static PROMPTS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// Remember the line a prompt ran.
pub fn remember(line: &str) {
    let line = line.trim();
    if line.is_empty() {
        return;
    }
    PROMPTS.with(|slot| {
        let mut lines = slot.borrow_mut();
        lines.insert(0, line.to_string());
        lines.truncate(KEPT);
    });
}

/// Forget every remembered line.
///
/// `history -c` clears this too — a line the user asked to be gone must not stay reachable by
/// coordinate.
pub fn forget() {
    PROMPTS.with(|slot| slot.borrow_mut().clear());
}

/// Every remembered line, newest first.
pub fn all() -> Vec<String> {
    PROMPTS.with(|slot| slot.borrow().clone())
}

/// The line `back` prompts ago, where 1 is the previous one.
///
/// Answers `None` past the end, which reads as an empty selection rather than an error — the rule
/// the rest of the coordinate machinery follows.
pub fn back(back: usize) -> Option<String> {
    match back {
        0 => None,
        n => PROMPTS.with(|slot| slot.borrow().get(n - 1).cloned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ring is bounded, newest first, and a blank line is not an event.
    #[test]
    fn the_ring_keeps_the_newest_ten() {
        forget();
        for n in 0..KEPT + 5 {
            remember(&format!("line-{n}"));
        }
        assert_eq!(all().len(), KEPT);
        assert_eq!(back(1), Some(format!("line-{}", KEPT + 4)));
        assert_eq!(back(KEPT + 1), None);
        assert_eq!(back(0), None);

        remember("   ");
        assert_eq!(back(1), Some(format!("line-{}", KEPT + 4)));
        forget();
        assert!(all().is_empty());
    }
}

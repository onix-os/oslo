//! What the Enter key does, which is not the same question at both of oslo's prompts.
//!
//! At a shell prompt Enter runs the line, and nothing here changes that: a shell where Enter did
//! not run the line would break every habit anyone has, and an unfinished command already gets a
//! continuation prompt. At a **Lua** prompt it is a real choice, because a Lua block is often
//! several lines and every Enter meaning "run this" interrupts writing one.
//!
//! # Why this is safe to turn on
//!
//! Ctrl+Enter and Alt+Enter always send, whatever Enter is set to do — they decode to one key on
//! purpose (see [`crate::term::keyboard`]). That matters more than it looks: **Ctrl+Enter does not
//! exist on a terminal without the kitty keyboard protocol.** In the legacy encoding Ctrl-M *is*
//! Enter, so the two cannot be told apart, and a prompt whose only send key was Ctrl+Enter would
//! be a prompt that never runs anything. Alt+Enter arrives in both encodings, so there is always a
//! way out of a block.

use std::sync::atomic::{AtomicBool, Ordering};

static ADDS_A_LINE: AtomicBool = AtomicBool::new(false);

/// Choose what Enter does: add a line to the block, or send it.
///
/// **A global rather than a field on the session**, and for once that is the honest shape: the
/// prompt changes language from a key handler that has no way to reach the session, and this
/// belongs to the language rather than to the line being edited. The REPL sets it again on every
/// switch, so toggling to Lua mid-line still gets the right answer for the next Enter.
pub fn set_enter_adds_a_line(yes: bool) {
    ADDS_A_LINE.store(yes, Ordering::Relaxed);
}

pub(super) fn adds_a_line() -> bool {
    ADDS_A_LINE.load(Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::super::{Key, Session, Step, set_enter_adds_a_line};
    use crate::edit::session::NoAssist;

    fn run(start: &str, keys: &[Key]) -> (Session, Vec<Step>) {
        let mut session = Session {
            vi: None,
            ..Session::new(start, start.chars().count())
        };
        let mut assist = NoAssist;
        let steps = keys
            .iter()
            .map(|k| session.apply(*k, &mut assist))
            .collect();
        (session, steps)
    }

    /// **Enter sends by default**, adds a line only where that was asked for, and the chord sends
    /// either way — which is the property the module docs turn on.
    #[test]
    fn enter_sends_unless_it_has_been_asked_to_add_a_line() {
        let (session, steps) = run("print(1)", &[Key::Accept]);
        assert_eq!(steps, vec![Step::Accept], "the default is to send");
        assert_eq!(session.buffer.text(), "print(1)");

        set_enter_adds_a_line(true);
        let (session, steps) = run("do", &[Key::Accept, Key::Char('x')]);
        assert!(
            matches!(steps[0], Step::Continue { .. }),
            "Enter should not have sent: {steps:?}"
        );
        assert_eq!(session.buffer.text(), "do\nx", "it added a line");

        // The chord sends regardless, which is the way out of the block.
        let (_, steps) = run("do\nend", &[Key::Alt('\r')]);
        assert_eq!(steps, vec![Step::Accept], "Ctrl/Alt+Enter always sends");

        set_enter_adds_a_line(false);
        let (_, steps) = run("print(1)", &[Key::Accept]);
        assert_eq!(steps, vec![Step::Accept], "and back to sending");
    }
}

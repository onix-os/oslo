//! Keys whose meaning *this prompt* decides, rather than the keymap.
//!
//! **Enter is not one of them, and that was a mistake worth recording.** It briefly inserted a
//! newline into the buffer so a Lua block could be typed across several lines. The editor edits one
//! line and draws one line, so the buffer held `\n` while the screen redrew the prompt over itself
//! — the block was invisible and the prompt appeared to stutter. Multi-line input is the *reader's*
//! job, not the editor's: `startup::read` accumulates lines and shows a continuation prompt, which
//! is how every REPL does it and how oslo already did it for shell.

use super::{Session, Step};
use crate::term::Key;

/// **Tab twice on an empty line switches language.**
///
/// Shift+Tab does it anywhere and is the key to reach for — but it only arrives on a terminal that
/// reports modifiers, and on one that does not there was no way to switch at all. This needs
/// nothing from the terminal: a plain Tab is a plain Tab everywhere. So it is always on, as the
/// third of three, because each of the other two can fail on a machine where nothing is obviously
/// wrong — Ctrl+Space is claimed by ibus and fcitx as the input-method switch.
///
/// **Why it needs a remembered flag when double-space needs no memory.** A space inserts itself, so
/// "the line is one space" *is* the record that the last key was a space. Tab inserts nothing, so
/// the only record that one was pressed is the flag carried here.
///
/// Only on an empty line, so Tab keeps its whole ordinary meaning the moment there is anything to
/// complete. What it costs is Tab on an empty prompt, which listed every name on `$PATH` — the same
/// trade a leading space already makes.
///
/// `None` means this Tab is not one of ours and completion should have it.
pub(super) fn tab(session: &mut Session, key: Key) -> Option<Step> {
    // Read and cleared on every key, so only two *consecutive* Tabs count.
    let armed = std::mem::replace(&mut session.tab_armed, false);
    if key != Key::ToggleScope || !session.buffer.text().is_empty() {
        return None;
    }
    if armed {
        return Some(Step::ToggleLanguage);
    }
    session.tab_armed = true;
    Some(Step::Continue { redraw: false })
}

#[cfg(test)]
mod tests {
    use super::super::{Key, Session, Step};
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

    /// **Tab twice on an empty line switches language**, which is the fallback for a terminal that
    /// cannot deliver Shift+Tab.
    ///
    /// The flag is needed because Tab inserts nothing: unlike double-space, the line itself carries
    /// no record that a Tab was pressed.
    #[test]
    fn a_second_tab_on_an_empty_line_switches_language() {
        // Always on: it is the one of the three that no terminal or input method can take away.
        let (_, steps) = run("", &[Key::ToggleScope, Key::ToggleScope]);
        assert_eq!(
            steps,
            vec![Step::Continue { redraw: false }, Step::ToggleLanguage],
            "the first arms, the second switches"
        );

        // **Only when they are consecutive.** Anything in between disarms it, or a Tab from five
        // keystroke ago would switch language when the line happened to be empty again.
        let (_, steps) = run(
            "",
            &[
                Key::ToggleScope,
                Key::Char('x'),
                Key::Backspace,
                Key::ToggleScope,
            ],
        );
        assert!(
            !steps.contains(&Step::ToggleLanguage),
            "a key in between should disarm: {steps:?}"
        );

        // **Not once there is something to complete.** Tab keeps its whole ordinary meaning the
        // moment the line is not empty.
        let (_, steps) = run("ls ", &[Key::ToggleScope, Key::ToggleScope]);
        assert!(
            !steps.contains(&Step::ToggleLanguage),
            "Tab on a non-empty line is completion: {steps:?}"
        );
    }
}

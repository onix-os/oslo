//! The two keys whose meaning *this prompt* decides, rather than the keymap.
//!
//! Both are here because both are the same shape of question — an ordinary key that means one
//! thing at a shell prompt and may mean another at a Lua one — and because both are settings with
//! a default that has to be argued for rather than assumed.
//!
//! # Enter
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

use super::{Session, Step};
use crate::term::Key;
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

static DOUBLE_TAB: AtomicBool = AtomicBool::new(true);

/// Whether Tab twice on an empty line switches language.
///
/// **On by default**, as the third of three, because each of the other two can fail on a machine
/// where nothing is obviously wrong: Shift+Tab needs the terminal to report a modifier, and
/// Alacritty without the kitty keyboard protocol does not; Ctrl+Space needs the terminal to *see*
/// it, and ibus or fcitx claims it as the input-method switch first. A plain Tab is a plain Tab
/// everywhere, so this is the one that cannot be taken away.
///
/// What it costs is Tab at an *empty* prompt, which otherwise offers every name on `$PATH` — a
/// listing nobody reads. Only on an empty line, so Tab keeps its whole ordinary meaning the moment
/// there is anything to complete. `$OSLO_DOUBLE_TAB=off` gives the empty-prompt listing back.
pub fn set_double_tab_toggles(yes: bool) {
    DOUBLE_TAB.store(yes, Ordering::Relaxed);
}

pub(super) fn double_tab_toggles() -> bool {
    DOUBLE_TAB.load(Ordering::Relaxed)
}

/// **Tab twice on an empty line switches language.**
///
/// Shift+Tab does it anywhere and is the key to reach for — but it only arrives on a terminal that
/// reports modifiers, and on one that does not there was no way to switch at all. This needs
/// nothing from the terminal: a plain Tab is a plain Tab everywhere.
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
    if key != Key::ToggleScope || !session.buffer.text().is_empty() || !double_tab_toggles() {
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
    use super::super::{Key, Session, Step, set_double_tab_toggles, set_enter_adds_a_line};
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
        let (_, steps) = run("do\nend", &[Key::Submit]);
        assert_eq!(steps, vec![Step::Accept], "Ctrl+Enter always sends");

        set_enter_adds_a_line(false);
        let (_, steps) = run("print(1)", &[Key::Accept]);
        assert_eq!(steps, vec![Step::Accept], "and back to sending");
    }

    /// **Tab twice on an empty line switches language**, which is the fallback for a terminal that
    /// cannot deliver Shift+Tab.
    ///
    /// The flag is needed because Tab inserts nothing: unlike double-space, the line itself carries
    /// no record that a Tab was pressed.
    #[test]
    fn a_second_tab_on_an_empty_line_switches_language() {
        // On by default: it is the one of the three that no terminal or input method can take
        // away, so it is not something a user should have to find a setting for.
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

        set_double_tab_toggles(false);
        let (_, steps) = run("", &[Key::ToggleScope, Key::ToggleScope]);
        assert!(
            !steps.contains(&Step::ToggleLanguage),
            "turned off, it is completion again: {steps:?}"
        );
        // Left at the real default: this is a process-wide flag, so a test that changed it and did
        // not put it back would decide the answer for whatever ran next.
        set_double_tab_toggles(true);
    }
}

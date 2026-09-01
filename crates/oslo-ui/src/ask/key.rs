//! `key` — wait for one keypress and say what it is called.
//!
//! **A binding is written by name**, and until this existed there was no way to learn a name
//! except by reading oslo's source: `oslo.keys["ctrl-g"]` fires and `oslo.keys["C-g"]` silently
//! never does, with nothing to say which spelling was which.
//!
//! A widget rather than a tool of its own, because it asks the same question every other widget
//! asks — *press something* — and because being one makes it useful twice. `k=$(ui key)` lets a
//! script read a single keypress, which nothing else here does.

use super::Answer;
use crate::term::{Key, Keys, Restore, Screen};

/// Wait for one keypress and answer with the name a binding would use for it.
///
/// Nothing is [`Answer::Cancelled`]: every key has a name, including the ones that would cancel
/// another widget. Escape is a key somebody may want to bind, so it is reported rather than obeyed
/// — which is the one way this widget deliberately breaks the family's habit.
pub fn key() -> Answer<String> {
    let Some(raw) = Restore::enter(Screen::Inline) else {
        return Answer::NoTerminal;
    };
    let mut keys = Keys::on(raw.fd());
    loop {
        let Some(pressed) = keys.read() else {
            return Answer::Cancelled;
        };
        // A redraw request is not a keypress, and answering with one would be answering a question
        // nobody asked. Wait for a real one.
        if matches!(pressed, Key::Resized | Key::Ignored) {
            continue;
        }
        return Answer::Given(pressed.name());
    }
}

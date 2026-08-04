//! `ui input` — one line, typed.
//!
//! Inline rather than full-screen: the question and the answer stay in the scrollback where you
//! can scroll back and see what you agreed to. A picker that takes the whole screen and restores
//! it leaves nothing behind, which is right for browsing history and wrong for a question a script
//! asked you.
//!
//! # What it is not
//!
//! It is not the line editor. There is no completion, no history, no syntax colour, and the
//! editing keys stop at the ones people reach for without thinking: the arrows, Home and End,
//! backspace and forward delete, and Ctrl-U. A script asking for a branch name does not need
//! oslo's whole `readline`, and hosting it here would mean two things to keep in step for the rest
//! of the shell's life.

use super::{Answer, show};
use crate::interactive::prompt::printed_width;
use crate::interactive::term::{Key, Keys, Restore};
use crate::interactive::theme;

/// How `input` is asked.
#[derive(Debug, Clone, Default)]
pub struct Input {
    /// Shown before the cursor.
    pub prompt: String,
    /// Greyed text shown while the line is empty. Not a value — pressing Enter on it answers the
    /// empty string, which is the difference between a placeholder and a default.
    pub placeholder: String,
    /// The line starts with this, and Enter on an untouched line answers it. Also the answer when
    /// there is no terminal.
    pub default: Option<String>,
    /// Draw `•` instead of the characters. For a password, which is the one case where the
    /// scrollback keeping what you typed is exactly wrong.
    pub password: bool,
    /// Refuse to answer with an empty line.
    pub required: bool,
}

/// Ask for one line.
pub fn input(spec: &Input) -> Answer<String> {
    let ui = theme::current().ui;
    let depth = theme::depth();

    let Some(raw) = Restore::enter(false) else {
        // No terminal. A default is the script's own answer for exactly this, and is why one of
        // these in a CI pipeline need not hang.
        return match &spec.default {
            Some(value) => Answer::Given(value.clone()),
            None => Answer::NoTerminal,
        };
    };

    let mut line: Vec<char> = spec.default.clone().unwrap_or_default().chars().collect();
    let mut cursor = line.len();
    let mut keys = Keys::on(raw.fd());
    // Everything this widget draws lives on one row, so a redraw is a carriage return and an
    // erase — no cursor arithmetic, and nothing that can drift out of step with the terminal.
    show("\r\x1b[K");

    loop {
        let shown: String = if spec.password {
            "•".repeat(line.len())
        } else {
            line.iter().collect()
        };
        let body = if line.is_empty() && !spec.placeholder.is_empty() {
            ui.muted.paint(&spec.placeholder, depth)
        } else {
            shown.clone()
        };
        let before: String = if spec.password {
            "•".repeat(cursor)
        } else {
            line[..cursor].iter().collect()
        };
        // `CSI 0 C` is a move of *one*, not none — so the caret sat a cell right of an empty
        // line and did not move when the first character was typed.
        let caret = printed_width(&spec.prompt) + printed_width(&before);
        show(&format!(
            "\r\x1b[K{}{}\r{}",
            ui.question.paint(&spec.prompt, depth),
            body,
            if caret > 0 {
                format!("\x1b[{caret}C")
            } else {
                String::new()
            }
        ));

        let Some(pressed) = keys.read() else {
            show("\r\x1b[K");
            return Answer::Cancelled;
        };
        match pressed {
            // An abort is a cancel here: there is an answer to decline either way.
            Key::Cancel | Key::Abort => {
                show("\r\x1b[K");
                return Answer::Cancelled;
            }
            Key::Accept => {
                let answer: String = line.iter().collect();
                if spec.required && answer.is_empty() {
                    // Say why, on the same row, and go on asking. Refusing silently reads as the
                    // key not having worked.
                    show(&format!(
                        "\r\x1b[K{}{}",
                        ui.question.paint(&spec.prompt, depth),
                        ui.error.paint("required", depth)
                    ));
                    continue;
                }
                // Erased, not echoed: the caller prints the answer to stdout and that is the one
                // record of it. Echoing here as well put the answer on screen twice, once from
                // each stream — and made `x=$(ui input)` leave a stray copy behind.
                show("\r\x1b[K");
                return Answer::Given(answer);
            }
            Key::Char(c) => {
                line.insert(cursor, c);
                cursor += 1;
            }
            Key::Backspace => {
                if cursor > 0 {
                    cursor -= 1;
                    line.remove(cursor);
                }
            }
            Key::Delete => {
                if cursor < line.len() {
                    line.remove(cursor);
                }
            }
            Key::Left => cursor = cursor.saturating_sub(1),
            Key::Right => cursor = (cursor + 1).min(line.len()),
            Key::Home | Key::PageUp => cursor = 0,
            Key::End | Key::PageDown => cursor = line.len(),
            Key::Clear => {
                line.clear();
                cursor = 0;
            }
            // Nothing here scrolls or toggles.
            Key::Up | Key::Down | Key::ToggleScope | Key::BackTab | Key::Ignored => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// With no terminal, a default is the answer and its absence is a distinct status — which is
    /// what lets the same script run under CI and at a prompt.
    #[test]
    fn without_a_terminal_the_default_answers() {
        let with = Input {
            default: Some("fallback".to_string()),
            ..Input::default()
        };
        // Under `cargo test` stdin is not a terminal, so this exercises the headless path.
        assert_eq!(input(&with), Answer::Given("fallback".to_string()));

        let without = Input::default();
        assert_eq!(input(&without), Answer::NoTerminal);
    }

    /// The status is the whole interface for `x=$(ui input) || exit`.
    #[test]
    fn the_statuses_are_distinct() {
        assert_eq!(Answer::Given(()).status(), 0);
        assert_eq!(Answer::<()>::Cancelled.status(), 1);
        assert_eq!(Answer::<()>::NoTerminal.status(), 2);
    }
}

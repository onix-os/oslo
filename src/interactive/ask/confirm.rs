//! `ui confirm` — yes or no, as an exit status.
//!
//! The answer is the status, not the output: `ui confirm "go on?" && do_it` is the shape every
//! script wants, and printing `yes` to stdout for the caller to compare against would be a worse
//! interface wearing the same clothes.
//!
//! Two buttons rather than a `[y/N]` prompt, because a button you can see is selected tells you
//! what Enter will do. `y` and `n` still work, since that is what fingers already know.

use super::{Answer, legend, show};
use crate::interactive::term::{Key, Keys, Restore};
use crate::interactive::theme;

/// How `confirm` is asked.
#[derive(Debug, Clone)]
pub struct Confirm {
    pub question: String,
    pub yes: String,
    pub no: String,
    /// Which button starts selected, and the answer when there is no terminal.
    ///
    /// Defaults to **no**. A confirm is asked before something you cannot undo, and a prompt that
    /// starts on `yes` turns a reflexive Enter into the destructive answer.
    pub default: bool,
}

impl Default for Confirm {
    fn default() -> Self {
        Confirm {
            question: "Are you sure?".to_string(),
            yes: "Yes".to_string(),
            no: "No".to_string(),
            default: false,
        }
    }
}

pub fn confirm(spec: &Confirm) -> Answer<bool> {
    let ui = theme::current().ui;
    let depth = theme::depth();

    let Some(raw) = Restore::enter(false) else {
        return Answer::Given(spec.default);
    };

    let mut yes = spec.default;
    let mut keys = Keys::on(raw.fd());

    loop {
        // The chosen button takes the accent as a *background*, which is what makes it read as a
        // button rather than as coloured text.
        let button = |label: &str, on: bool| {
            let style = if on {
                theme::Style {
                    bg: ui.accent.fg,
                    fg: None,
                    bold: true,
                    ..theme::Style::default()
                }
            } else {
                ui.muted
            };
            style.paint(&format!(" {label} "), depth)
        };
        show(&format!(
            "\r\x1b[K{}  {}  {}   {}",
            ui.question.paint(&spec.question, depth),
            button(&spec.yes, yes),
            button(&spec.no, !yes),
            legend(&[("←→", "choose"), ("enter", "confirm")])
        ));

        let Some(pressed) = keys.read() else {
            show("\r\x1b[K");
            return Answer::Cancelled;
        };
        match pressed {
            Key::Cancel => {
                show("\r\x1b[K");
                return Answer::Cancelled;
            }
            Key::Accept => {
                let chosen = if yes { &spec.yes } else { &spec.no };
                show(&format!(
                    "\r\x1b[K{}  {}\r\n",
                    ui.question.paint(&spec.question, depth),
                    if yes { ui.done } else { ui.error }.paint(chosen, depth)
                ));
                return Answer::Given(yes);
            }
            // The letters, because that is what people type without looking.
            Key::Char('y') | Key::Char('Y') => yes = true,
            Key::Char('n') | Key::Char('N') => yes = false,
            Key::Left | Key::Right | Key::ToggleScope | Key::BackTab => yes = !yes,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A confirm defaults to **no**: it is asked before something irreversible, and a reflexive
    /// Enter must not be the destructive answer.
    #[test]
    fn the_default_is_no() {
        assert!(!Confirm::default().default);
    }

    /// With no terminal the default is the answer, so a script that confirms still runs headless.
    #[test]
    fn without_a_terminal_the_default_answers() {
        assert_eq!(confirm(&Confirm::default()), Answer::Given(false));
        let yes = Confirm {
            default: true,
            ..Confirm::default()
        };
        assert_eq!(confirm(&yes), Answer::Given(true));
    }
}

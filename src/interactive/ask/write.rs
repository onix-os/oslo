//! `ui write` — several lines, typed.
//!
//! [`super::input`] with a second dimension, and the difference is entirely in what ends it: Enter
//! makes a new line, and **Ctrl-D** is done. That is the rule every multi-line prompt in Unix has
//! used since `mail(1)`, and picking a different one here would only make people press Enter twice
//! and wonder.
//!
//! Drawn in place below the header, like the list widgets, so the transcript above is untouched
//! and the finished text stays where it was typed.

use super::{Answer, FOOTER_ROWS, Inline, footer, with_caret};
use crate::interactive::dropdown::width::{terminal_cols, terminal_rows, truncate_to_width};
use crate::interactive::term::{Key, Keys, Restore};
use crate::interactive::theme;

/// How `write` is asked.
#[derive(Debug, Clone, Default)]
pub struct Write {
    pub header: String,
    pub placeholder: String,
    /// The text starts as this, and it is the answer when there is no terminal.
    pub default: Option<String>,
}

pub fn write(spec: &Write) -> Answer<String> {
    let ui = theme::current().ui;
    let depth = theme::depth();

    let Some(raw) = Restore::enter(false) else {
        return match &spec.default {
            Some(value) => Answer::Given(value.clone()),
            None => Answer::NoTerminal,
        };
    };

    // A cursor in two dimensions: which line, and where in it. Kept as characters rather than
    // bytes so moving across a multibyte character is one press.
    let mut lines: Vec<Vec<char>> = spec
        .default
        .clone()
        .unwrap_or_default()
        .split('\n')
        .map(|l| l.chars().collect())
        .collect();
    if lines.is_empty() {
        lines.push(Vec::new());
    }
    let mut row = lines.len() - 1;
    let mut col = lines[row].len();
    let mut keys = Keys::on(raw.fd());
    let mut panel = Inline::new();

    loop {
        let cols = terminal_cols();
        let room = cols.saturating_sub(3);
        // The document scrolls under a window, like every other list here. Without this a document
        // longer than the terminal walked off the top and the redraw painted over the transcript.
        let chrome = FOOTER_ROWS + usize::from(!spec.header.is_empty());
        let window = terminal_rows().saturating_sub(chrome + 1).max(1);
        let top = row.saturating_sub(window.saturating_sub(1));

        let mut frame = String::new();
        if !spec.header.is_empty() {
            frame.push_str(&format!(
                "\r\n\r\x1b[K{}",
                ui.question
                    .paint(&truncate_to_width(&spec.header, room), depth)
            ));
        }
        let empty = lines.len() == 1 && lines[0].is_empty();
        for (index, line) in lines.iter().enumerate().skip(top).take(window) {
            let raw_text: String = line.iter().collect();
            // The caret is drawn into the row it is on, not positioned afterwards — the real
            // cursor is hidden, and a block that is part of the frame cannot drift out of step
            // with the text.
            let text: String = if empty && !spec.placeholder.is_empty() {
                let mut chars = spec.placeholder.chars();
                let first = chars.next().unwrap_or(' ');
                format!(
                    "{}{}",
                    with_caret(&first.to_string(), 0),
                    ui.muted.paint(chars.as_str(), depth)
                )
            } else if index == row {
                with_caret(&truncate_to_width(&raw_text, room), col)
            } else {
                truncate_to_width(&raw_text, room)
            };
            // `┃` in the accent, which is gum's prompt for a multi-line field and the same colour
            // every other cursor in this module uses.
            frame.push_str(&format!(
                "\r\n\r\x1b[K{} {}",
                if index == row { ui.accent } else { ui.muted }.paint("┃", depth),
                text
            ));
        }
        let bottom = footer(
            &frame,
            &[
                ("enter", "new line"),
                ("ctrl-d", "submit"),
                ("esc", "cancel"),
            ],
        );
        frame.push_str(&bottom);
        panel.draw(&frame);

        let Some(pressed) = keys.read() else {
            panel.close();
            return Answer::Cancelled;
        };

        match pressed {
            // An abort is a cancel here: there is an answer to decline either way.
            Key::Cancel | Key::Abort => {
                panel.close();
                return Answer::Cancelled;
            }
            // Ctrl-D on an empty document is a cancel rather than an empty answer, matching what
            // the same key does at a shell prompt.
            Key::Delete => {
                let text = lines
                    .iter()
                    .map(|l| l.iter().collect::<String>())
                    .collect::<Vec<_>>()
                    .join("\n");
                panel.close();
                return if text.is_empty() {
                    Answer::Cancelled
                } else {
                    Answer::Given(text)
                };
            }
            Key::Accept => {
                let tail: Vec<char> = lines[row].split_off(col);
                lines.insert(row + 1, tail);
                row += 1;
                col = 0;
            }
            Key::Char(c) => {
                lines[row].insert(col, c);
                col += 1;
            }
            Key::Backspace => {
                if col > 0 {
                    col -= 1;
                    lines[row].remove(col);
                } else if row > 0 {
                    // Joining a line to the one above is what backspace at column zero means.
                    let joined = lines.remove(row);
                    row -= 1;
                    col = lines[row].len();
                    lines[row].extend(joined);
                }
            }
            Key::Left => {
                if col > 0 {
                    col -= 1;
                } else if row > 0 {
                    row -= 1;
                    col = lines[row].len();
                }
            }
            Key::Right => {
                if col < lines[row].len() {
                    col += 1;
                } else if row + 1 < lines.len() {
                    row += 1;
                    col = 0;
                }
            }
            Key::Up => {
                row = row.saturating_sub(1);
                col = col.min(lines[row].len());
            }
            Key::Down => {
                row = (row + 1).min(lines.len() - 1);
                col = col.min(lines[row].len());
            }
            Key::Home => col = 0,
            Key::End => col = lines[row].len(),
            Key::Clear => {
                lines[row].clear();
                col = 0;
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// With no terminal a default answers, and its absence is the distinct status.
    #[test]
    fn without_a_terminal_the_default_answers() {
        let with = Write {
            default: Some("line one\nline two".to_string()),
            ..Write::default()
        };
        assert_eq!(
            write(&with),
            Answer::Given("line one\nline two".to_string())
        );
        assert_eq!(write(&Write::default()), Answer::NoTerminal);
    }
}

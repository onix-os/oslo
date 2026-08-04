//! `ui write` — several lines, typed.
//!
//! [`super::input`] with a second dimension, and the difference is entirely in what ends it: Enter
//! makes a new line, and **Ctrl-D** is done. That is the rule every multi-line prompt in Unix has
//! used since `mail(1)`, and picking a different one here would only make people press Enter twice
//! and wonder.
//!
//! Drawn in place below the header, like the list widgets, so the transcript above is untouched
//! and the finished text stays where it was typed.

use super::{Answer, legend, show};
use crate::interactive::dropdown::width::terminal_cols;
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
    let mut drawn = 0usize;

    loop {
        let mut frame = String::new();
        if drawn > 0 {
            frame.push_str(&format!("\x1b[{drawn}A"));
        }
        if !spec.header.is_empty() {
            frame.push_str(&format!(
                "\r\x1b[K{}\r\n",
                ui.question.paint(&spec.header, depth)
            ));
        }
        let empty = lines.len() == 1 && lines[0].is_empty();
        for (index, line) in lines.iter().enumerate() {
            let text: String = if empty && !spec.placeholder.is_empty() {
                ui.muted.paint(&spec.placeholder, depth)
            } else {
                line.iter().collect()
            };
            frame.push_str(&format!(
                "\r\x1b[K{} {}\r\n",
                ui.muted.paint(if index == row { "▎" } else { " " }, depth),
                text
            ));
        }
        frame.push_str(&format!(
            "\r\x1b[K{}",
            legend(&[("enter", "new line"), ("ctrl-d", "done"), ("esc", "cancel")])
        ));
        show(&frame);
        drawn = lines.len() + 1 + usize::from(!spec.header.is_empty());
        // Put the cursor where the caret is, so a terminal's own cursor tracks the text.
        let up = lines.len() - row;
        let across = 2 + col.min(terminal_cols().saturating_sub(3));
        show(&format!("\x1b[{up}A\r\x1b[{across}C"));

        let Some(pressed) = keys.read() else {
            erase(drawn, row, lines.len());
            return Answer::Cancelled;
        };
        // Every branch below leaves the cursor wherever it was; the next frame moves it.
        show(&format!("\x1b[{}B", lines.len() - row));

        match pressed {
            Key::Cancel => {
                erase(drawn, 0, 0);
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
                erase(drawn, 0, 0);
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

/// Erase the rows this widget printed.
fn erase(rows: usize, _row: usize, _total: usize) {
    if rows == 0 {
        return;
    }
    let mut out = format!("\x1b[{rows}A");
    for _ in 0..rows {
        out.push_str("\r\x1b[K\r\n");
    }
    out.push_str(&format!("\x1b[{rows}A"));
    show(&out);
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

//! `ui pager` — read something long, then leave the screen as it was.
//!
//! Full-screen, unlike every other widget here except the history finder, and for the same reason:
//! you came to look at a lot of text, and the alternate screen means the transcript behind it is
//! untouched when you leave.
//!
//! # Why not just run `less`
//!
//! Because a script cannot count on one being installed, and because `$PAGER` may be anything at
//! all — including something that ignores its arguments or waits for input the script cannot give.
//! A shell that ships prompts should be able to show a paragraph without a dependency.
//!
//! It is deliberately not a `less` replacement: no search, no marks, no half-page scroll. Up and
//! down, page up and down, home and end, and `q`.

use super::chrome::legend_text;
use super::{Answer, show};
use crate::ui::dropdown::width::{terminal_cols, terminal_rows, truncate_to_width};
use crate::ui::term::{Key, Keys, Restore, Screen};
use crate::ui::theme;

/// What to show, and what to call it.
#[derive(Debug, Clone, Default)]
pub struct Pager {
    pub title: String,
    pub text: String,
    /// Wrap long lines rather than cutting them. Off matches `less -S`, which is right for tables
    /// and logs; on is right for prose.
    pub wrap: bool,
    /// The legend, the border, the screen and where on it. See `super::chrome`.
    pub chrome: super::chrome::Chrome,
}

/// Show `spec` until it is dismissed. There is nothing to answer with, so the outcome is only
/// whether there was a terminal at all.
pub fn pager(spec: &Pager) -> Answer<()> {
    let ui = theme::current().ui;
    let depth = theme::depth();

    let Some(raw) = Restore::enter(Screen::Alternate) else {
        return Answer::NoTerminal;
    };

    let cols = terminal_cols();
    let lines: Vec<String> = if spec.wrap {
        spec.text.lines().flat_map(|l| wrapped(l, cols)).collect()
    } else {
        spec.text.lines().map(str::to_string).collect()
    };

    let mut top = 0usize;
    let mut keys = Keys::on(raw.fd());

    loop {
        let rows = terminal_rows();
        // The title bar above, and the footer — a blank row and the keys — below.
        let window = rows.saturating_sub(1 + spec.chrome.legend_rows()).max(1);
        let last = lines.len().saturating_sub(window);
        top = top.min(last);

        let mut frame = String::from("\x1b[H");
        let seen = (top + window).min(lines.len());
        let position = if lines.len() <= window {
            "all".to_string()
        } else {
            format!("{}%", seen * 100 / lines.len().max(1))
        };
        frame.push_str(&format!(
            "\x1b[2K{}  {}\r\n",
            ui.question.paint(&spec.title, depth),
            ui.muted.paint(&position, depth)
        ));
        for row in 0..window {
            let text = lines
                .get(top + row)
                .map(|l| truncate_to_width(l, cols))
                .unwrap_or_default();
            frame.push_str(&format!("\x1b[2K{text}\r\n"));
        }
        // The pager is full-width by construction, so its rule spans the terminal rather than the
        // widest row — measuring the content would give a ragged line under a ragged document.
        frame.push_str(&format!(
            "\x1b[2K{}\r\n\x1b[2K{}",
            ui.muted.paint("- ".repeat(cols / 2).trim_end(), depth),
            legend_text(&[("↑↓", "scroll"), ("q", "quit")])
        ));
        show(&frame);

        let Some(pressed) = keys.read() else {
            return Answer::Given(());
        };
        match pressed {
            // `q`, Esc and Enter are all "finished reading", which is the pager's success — there
            // is nothing here to answer, so declining to answer is not a thing you can do.
            Key::Cancel | Key::Accept | Key::Char('q') => return Answer::Given(()),
            // Ctrl-C is not that. A script running `ui pager … || cleanup` should see the abort.
            Key::Abort => return Answer::Cancelled,
            Key::Up => top = top.saturating_sub(1),
            Key::Down => top = (top + 1).min(last),
            Key::PageUp => top = top.saturating_sub(window),
            Key::PageDown | Key::Char(' ') => top = (top + window).min(last),
            Key::Home => top = 0,
            Key::End => top = last,
            _ => {}
        }
    }
}

/// The byte offset at which `text` has filled `width` cells.
///
/// Character by character, because a byte offset into the middle of one would panic and because a
/// wide character occupies two cells while being one character.
fn cells(text: &str, width: usize) -> usize {
    let mut used = 0usize;
    for (offset, c) in text.char_indices() {
        let w = crate::ui::prompt::printed_width(&c.to_string());
        if used + w > width {
            return offset;
        }
        used += w;
    }
    text.len()
}

/// One line broken to fit `width`, on a space where there is one.
///
/// Word-aware because breaking mid-word is what makes wrapped prose unreadable — and falling back
/// to a hard break is what stops a single long token from being dropped.
fn wrapped(line: &str, width: usize) -> Vec<String> {
    if width == 0 || crate::ui::prompt::printed_width(line) <= width {
        return vec![line.to_string()];
    }
    let mut out = Vec::new();
    let mut current = String::new();
    for word in line.split(' ') {
        let would = crate::ui::prompt::printed_width(&current)
            + usize::from(!current.is_empty())
            + crate::ui::prompt::printed_width(word);
        if !current.is_empty() && would > width {
            out.push(std::mem::take(&mut current));
        }
        if crate::ui::prompt::printed_width(word) > width {
            // A single token wider than the screen: hard-break it rather than lose it. Not
            // `truncate_to_width`, which marks what it cut with an ellipsis — right for a column
            // that has to fit, wrong here, where the rest of the word is on the next row.
            let mut rest = word;
            while crate::ui::prompt::printed_width(rest) > width {
                let cut = cells(rest, width);
                out.push(rest[..cut].to_string());
                rest = &rest[cut..];
            }
            current = rest.to_string();
            continue;
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_a_terminal_there_is_nothing_to_page() {
        assert_eq!(
            pager(&Pager {
                text: "anything".to_string(),
                ..Pager::default()
            }),
            Answer::NoTerminal
        );
    }

    #[test]
    fn a_short_line_is_not_wrapped() {
        assert_eq!(wrapped("short", 20), vec!["short"]);
    }

    /// Wrapping happens on a space, because breaking mid-word is what makes prose unreadable.
    #[test]
    fn wrapping_breaks_on_spaces() {
        assert_eq!(
            wrapped("one two three four", 9),
            vec!["one two", "three", "four"]
        );
    }

    /// A token wider than the screen is broken rather than dropped — the case a naive
    /// break-on-space silently loses.
    #[test]
    fn an_over_long_token_is_broken_rather_than_lost() {
        let out = wrapped("aaaaaaaaaa", 4);
        assert_eq!(out.concat(), "aaaaaaaaaa", "no characters may go missing");
        assert!(out.iter().all(|l| l.len() <= 4), "{out:?}");
    }

    #[test]
    fn a_zero_width_screen_does_not_loop_forever() {
        assert_eq!(wrapped("text", 0), vec!["text"]);
    }
}

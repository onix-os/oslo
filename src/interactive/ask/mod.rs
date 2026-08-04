//! Asking the person at the terminal something, from a script.
//!
//! This is oslo's answer to [gum](https://github.com/charmbracelet/gum): the widgets a shell
//! script needs to be interactive, available to both languages the shell reads. In shell they are
//! the `ui` builtin; in Lua they are `oslo.ui`. Both call the same code, so a prompt looks the
//! same whichever language asked for it.
//!
//! ```sh
//! name=$(ui input --placeholder "your name")
//! ui confirm "delete everything?" && rm -rf "$target"
//! branch=$(git branch | ui filter --header "check out")
//! ```
//!
//! ```lua
//! local name = oslo.ui.input{placeholder = "your name"}
//! if oslo.ui.confirm("delete everything?") then ... end
//! ```
//!
//! # The result goes to stdout, everything else to stderr
//!
//! A widget is a question, and the answer is the script's data. `name=$(ui input)` has to capture
//! the name and nothing else, so the prompt, the list, the cursor and the key legend are all
//! written to stderr — the same reason `read -p` puts its prompt there. That single rule is what
//! makes these composable in a pipeline instead of merely usable at a prompt.
//!
//! # Cancelling is a status, not an answer
//!
//! Esc and Ctrl-C exit non-zero and print nothing. A script can therefore write
//! `x=$(ui input) || exit` and mean it, where a widget that returned an empty string on cancel
//! would make "cancelled" and "typed nothing" indistinguishable — and gum gets this right for the
//! same reason.
//!
//! # No terminal, no widget
//!
//! With stdin not a terminal every widget refuses rather than blocking on a pipe that will never
//! deliver a keypress. `--default` says what to answer in that case, which is what makes a script
//! using these still work under CI.

mod choose;
mod confirm;
mod file;
mod format;
mod input;
mod join;
mod log;
mod pager;
mod spin;
mod style;
mod table;
mod write;

pub use choose::{Choice, choose, filter};
pub use confirm::{Confirm, confirm};
pub use file::{Browse, Want, file};
pub use format::{As, format};
pub use input::{Input, input};
pub use join::{Align, horizontal, vertical};
pub use log::{Entry, Level, line};
pub use pager::{Pager, pager};
pub use spin::{Spin, spin};
pub use style::{Border, Styling, style};
pub use table::{Table, parse as parse_table, table};
pub use write::{Write, write};

/// What a widget answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer<T> {
    /// The person answered.
    Given(T),
    /// Esc or Ctrl-C. Non-zero status, no output.
    Cancelled,
    /// There was no terminal to ask on, and no default was supplied.
    NoTerminal,
}

impl<T> Answer<T> {
    /// The exit status a script should see. gum's convention, and the one that makes
    /// `x=$(ui input) || exit` correct.
    pub fn status(&self) -> i32 {
        match self {
            Answer::Given(_) => 0,
            Answer::Cancelled => 1,
            Answer::NoTerminal => 2,
        }
    }

    pub fn given(self) -> Option<T> {
        match self {
            Answer::Given(value) => Some(value),
            _ => None,
        }
    }
}

/// Write to stderr, where a prompt belongs. See the module note.
pub(crate) fn show(text: &str) {
    use std::io::Write;
    let mut err = std::io::stderr();
    let _ = err.write_all(text.as_bytes());
    let _ = err.flush();
}

/// Every inline widget draws the same way, and this is it.
///
/// A [`crate::interactive::paint::Panel`] plus the two rules that were got wrong when each widget
/// had its own copy of this:
///
/// * **rows are reserved before they are drawn**, so any scrolling happens while the cursor is
///   still accounted for. Drawing first and walking back up eats the caller's transcript one row
///   per keypress — see the module note on `paint`;
/// * **the row count is the number of `\r\n` written**, taken from the frame itself rather than
///   recomputed. Every widget used to compute it a second time and every one of them was off by
///   one, because the last row is written without a newline.
pub(crate) struct Inline {
    panel: crate::interactive::paint::Panel,
}

impl Inline {
    pub(crate) fn new() -> Inline {
        Inline {
            // Column 0: an inline widget starts at the beginning of the row below the prompt, and
            // there is nothing to the left of it to come back to.
            panel: crate::interactive::paint::Panel::at(0),
        }
    }

    /// Draw `frame`, whose rows are separated by `\r\n`.
    ///
    /// The count comes from the frame, so it cannot disagree with what was printed.
    pub(crate) fn draw(&mut self, frame: &str) {
        let rows = frame.matches("\r\n").count();
        show(&self.panel.draw(frame, rows));
    }

    /// Erase everything drawn, exactly.
    pub(crate) fn close(&mut self) {
        show(&self.panel.close());
    }
}

/// The bottom of a widget: a dashed rule, then the keys.
///
/// The rule is not decoration. The rows above it are the thing you are answering — a list, a
/// document, a pair of buttons — and the row below is a note *about* the widget. Run together they
/// read as one block and the eye has to work out which part is which.
///
/// **The rule is as wide as the widest row above it**, which is why this takes the frame rather
/// than a width: measured from what was actually drawn, it cannot fall out of step with the
/// content the way a width passed in by each caller would. `- ` rather than `─`, so it reads as a
/// tear-off line rather than as a border the widget does not have.
///
/// One helper rather than each widget appending its own, because that is exactly the kind of
/// detail that drifts: `confirm` had the question, the buttons and the keys all on one line while
/// its siblings had them on three.
pub(crate) fn footer(frame: &str, keys: &[(&str, &str)]) -> String {
    let ui = crate::interactive::theme::current().ui;
    let depth = crate::interactive::theme::depth();
    // As wide as the content and never wider, ending on a dash. The pattern is cut to the width
    // and then any trailing space dropped — turning that space into a dash instead gave `- - - --`
    // on an even-width row, which reads as a typo.
    let width = widest(frame);
    let rule: String = "- ".repeat(width.div_ceil(2)).chars().take(width).collect();
    let rule = rule.trim_end();
    format!(
        "\r\n\r\x1b[K{}\r\n\r\x1b[K{}",
        ui.muted.paint(rule, depth),
        legend(keys)
    )
}

/// How many rows [`footer`] draws, for a caller sizing its window.
pub(crate) const FOOTER_ROWS: usize = 2;

/// The printed width of the widest row in `frame`.
///
/// Rows are separated by `\r\n` and each carries its own `\r` and erase escape; those are
/// stripped before measuring, because they are instructions rather than anything on screen.
/// [`crate::interactive::prompt::printed_width`] already ignores colour.
fn widest(frame: &str) -> usize {
    frame
        .split("\r\n")
        .map(|row| {
            let row = row.trim_start_matches('\r');
            let row = row.strip_prefix("\x1b[K").unwrap_or(row);
            crate::interactive::prompt::printed_width(row)
        })
        .max()
        .unwrap_or(0)
}

/// The key legend along the bottom of a widget.
///
/// Always shown, and dim. A prompt whose keys you have to guess is one you leave by closing the
/// window; the row a legend costs is cheaper than that once.
///
/// `key what` pairs joined by ` • `, which is gum's separator and readable enough that the pairs
/// do not run together at a glance.
pub(crate) fn legend(keys: &[(&str, &str)]) -> String {
    let ui = crate::interactive::theme::current().ui;
    let depth = crate::interactive::theme::depth();
    let parts: Vec<String> = keys
        .iter()
        .map(|(key, what)| {
            format!(
                "{} {}",
                ui.accent.paint(key, depth),
                ui.muted.paint(what, depth)
            )
        })
        .collect();
    parts.join(&ui.muted.paint(" • ", depth))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Escapes stripped, so a test can assert on what is on screen.
    fn plain(rendered: &str) -> String {
        let mut out = String::new();
        let mut chars = rendered.chars().peekable();
        while let Some(c) = chars.next() {
            if c != '\x1b' {
                out.push(c);
                continue;
            }
            if chars.peek() == Some(&'[') {
                chars.next();
                for c in chars.by_ref() {
                    if c.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
        }
        out
    }

    /// The rule is measured from the frame, not from a number the caller passed — so it cannot
    /// disagree with what was drawn.
    #[test]
    fn the_rule_matches_the_widest_row() {
        let frame = "\r\n\r\x1b[Kshort\r\n\r\x1b[Kmuch much longer row";
        let rule = plain(&footer(frame, &[("q", "quit")]))
            .lines()
            .find(|l| l.contains('-'))
            .unwrap_or_default()
            .trim()
            .to_string();
        // Never wider than the content, and within one cell of it — a rule that overhangs the
        // thing it is under looks like a mistake, one that stops a cell short does not.
        let content = "much much longer row".len();
        let drawn = crate::interactive::prompt::printed_width(&rule);
        assert!(
            drawn <= content && drawn + 1 >= content,
            "{drawn} vs {content}: {rule:?}"
        );
        assert!(rule.ends_with('-'), "{rule:?}");
        assert!(!rule.contains("--"), "doubled dash: {rule:?}");
    }

    /// Colour must not count toward the width, or a styled row makes the rule too long.
    #[test]
    fn colour_does_not_widen_the_rule() {
        let plain_frame = "\r\n\r\x1b[Kabcd";
        let painted = "\r\n\r\x1b[K\x1b[1;31mabcd\x1b[0m";
        assert_eq!(widest(plain_frame), widest(painted));
        assert_eq!(widest(plain_frame), 4);
    }

    /// The erase escape and the carriage return are instructions, not content.
    #[test]
    fn the_row_prefix_is_not_measured() {
        assert_eq!(widest("\r\n\r\x1b[Kab"), 2);
        assert_eq!(widest(""), 0);
    }

    /// Two rows: the rule and the keys. Anything sizing a window depends on that being exact.
    #[test]
    fn the_footer_is_two_rows() {
        let drawn = footer("\r\n\r\x1b[Kx", &[("q", "quit")]);
        assert_eq!(drawn.matches("\r\n").count(), FOOTER_ROWS);
    }
}

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
pub mod chrome;
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
    /// The border, the placement and the screen this draws on. Default is exactly what every
    /// widget did before `chrome` existed: no border, top-left, inline.
    chrome: chrome::Chrome,
    /// Whether the alternate screen has been entered and still needs leaving.
    took_the_screen: bool,
}

impl Inline {
    /// An inline widget wrapped in `chrome`.
    ///
    /// Entering the alternate screen happens here rather than on the first draw, so the caller's
    /// transcript is saved before anything is written over it.
    pub(crate) fn with_chrome(chrome: chrome::Chrome) -> Inline {
        let took_the_screen = chrome.fullscreen;
        if took_the_screen {
            chrome::enter_fullscreen();
        }
        Inline {
            // Column 0: an inline widget starts at the beginning of the row below the prompt, and
            // there is nothing to the left of it to come back to.
            panel: crate::interactive::paint::Panel::at(0),
            chrome,
            took_the_screen,
        }
    }

    /// Draw `frame`, whose rows are separated by `\r\n`.
    ///
    /// The count comes from the frame *after* it has been wrapped, so a border's two extra rows are
    /// reserved and erased like any others. Computing it from the unwrapped frame is how a bordered
    /// widget would eat two rows of the transcript per keystroke.
    pub(crate) fn draw(&mut self, frame: &str, keys: &[(&str, &str)]) {
        let frame = self.chrome.wrap(frame, keys);
        let rows = frame.matches("\r\n").count();
        // Vertical placement is blank rows above, and only on a screen this widget owns. Inline,
        // pushing the frame down would scroll the caller's transcript rather than move anything.
        let margin = self.chrome.top_margin(rows + 1);
        let frame = match margin {
            0 => frame,
            n => format!("{}{frame}", "\r\n\r\x1b[K".repeat(n)),
        };
        let rows = frame.matches("\r\n").count();
        show(&self.panel.draw(&frame, rows));
    }

    /// Erase everything drawn, exactly.
    pub(crate) fn close(&mut self) {
        show(&self.panel.close());
        // Leaving puts the transcript back as it was, which is the whole reason to have taken a
        // second screen rather than clearing this one.
        if self.took_the_screen {
            chrome::leave_fullscreen();
            self.took_the_screen = false;
        }
    }
}

impl Drop for Inline {
    /// The screen goes back even if the widget was dropped without closing — a panic, an early
    /// return, a `?`. A shell left on the alternate screen is one whose scrollback has vanished,
    /// and the user has no way to get it back.
    fn drop(&mut self) {
        if self.took_the_screen {
            chrome::leave_fullscreen();
        }
    }
}

/// `text` with a block cursor drawn on the character at `at`.
///
/// The real terminal cursor is hidden for every widget — an inline one repaints its whole block on
/// each keystroke and the cursor is dragged across all of it — so a text field has to draw its own.
/// This is what bubbletea does for gum, and it is better than positioning the real one for a
/// reason beyond flicker: the caret is *part of the frame*, so it cannot end up a cell out of step
/// with the text the way a separate `ESC [ n C` can.
///
/// At the end of the text the block sits on a space, which is how you can tell "typing at the end"
/// from "on the last character".
pub(crate) fn with_caret(text: &str, at: usize) -> String {
    let ui = crate::interactive::theme::current().ui;
    let depth = crate::interactive::theme::depth();
    let block = crate::interactive::theme::Style {
        reverse: true,
        ..crate::interactive::theme::Style::default()
    };
    let chars: Vec<char> = text.chars().collect();
    let at = at.min(chars.len());
    let before: String = chars[..at].iter().collect();
    let under = chars.get(at).copied().unwrap_or(' ');
    let after: String = chars.iter().skip(at + 1).collect();
    let _ = ui;
    format!(
        "{}{}{}",
        before,
        block.paint(&under.to_string(), depth),
        after
    )
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

    /// The caret is part of the text, so it can never be a cell out of step with it.
    #[test]
    fn the_caret_marks_the_character_it_is_on() {
        // Reverse video around exactly one character, and the rest untouched.
        let drawn = with_caret("abc", 1);
        assert!(drawn.contains("\x1b[7m"), "not reversed: {drawn:?}");
        assert_eq!(plain(&drawn), "abc", "the text changed: {drawn:?}");
    }

    /// Past the end it sits on a space, which is how "typing at the end" looks different from
    /// "on the last character".
    #[test]
    fn at_the_end_the_caret_is_a_space() {
        let drawn = with_caret("ab", 2);
        assert_eq!(plain(&drawn), "ab ", "{drawn:?}");
        // And an index past even that cannot panic — a cursor arriving from shell code is a
        // number somebody could have written.
        assert_eq!(plain(&with_caret("ab", 99)), "ab ");
        assert_eq!(plain(&with_caret("", 0)), " ");
    }

    /// A multibyte character is one cell, not one byte — slicing by byte would panic on it.
    #[test]
    fn the_caret_counts_characters_not_bytes() {
        assert_eq!(plain(&with_caret("héllo", 1)), "héllo");
        assert_eq!(plain(&with_caret("→x", 0)), "→x");
    }
}

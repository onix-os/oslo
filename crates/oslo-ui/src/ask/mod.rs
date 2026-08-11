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
pub mod look;
mod pager;
mod spin;
mod style;
mod table;
mod write;

use crate::paint::{SYNC_BEGIN, SYNC_END};
use crate::term::Pressed;
pub use choose::{Choice, Pick, choose, filter, pick_or_create};
pub use confirm::{Confirm, confirm};
pub use file::{Browse, Want, file};
pub use format::{As, format};
pub use input::{Input, input};
pub use join::{Align, horizontal, vertical};
pub use log::{Entry, Level, line};
pub use look::{Look, Preset, Where};
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
    /// Map the value, keeping the status. Lets `input`'s single string be reported by the same
    /// code that reports `choose`'s list.
    ///
    /// Here rather than beside its caller in the `ui` builtin: an inherent `impl` has to live in
    /// the crate that owns the type, and this one owns `Answer`.
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Answer<U> {
        match self {
            Answer::Given(value) => Answer::Given(f(value)),
            Answer::Cancelled => Answer::Cancelled,
            Answer::NoTerminal => Answer::NoTerminal,
        }
    }

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
/// A [`crate::paint::Panel`] plus the two rules that were got wrong when each widget
/// had its own copy of this:
///
/// * **rows are reserved before they are drawn**, so any scrolling happens while the cursor is
///   still accounted for. Drawing first and walking back up eats the caller's transcript one row
///   per keypress — see the module note on `paint`;
/// * **the row count is the number of `\r\n` written**, taken from the frame itself rather than
///   recomputed. Every widget used to compute it a second time and every one of them was off by
///   one, because the last row is written without a newline.
pub(crate) struct Inline {
    panel: crate::paint::Panel,
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
            panel: crate::paint::Panel::at(0),
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
        // **One atomic update.** A frame is reserve-rows, move, erase, redraw — dozens of writes
        // that the terminal is otherwise free to render halfway through. That is what tearing is,
        // and on a list it reads as the rows flickering or jumping. It showed up the moment the
        // scanner made these redraw on a timer rather than only on a keystroke, but it was always
        // there on a fast typist.
        //
        // DEC mode 2026: a terminal that understands it buffers everything until the matching end
        // and presents the result in one go; one that does not ignores both, so this costs nothing
        // anywhere. The finder has drawn this way from the start — see `finder::render`.
        show(&format!(
            "{SYNC_BEGIN}{}{SYNC_END}",
            self.panel.draw(&frame, rows)
        ));
    }

    pub(crate) fn show_legend(&mut self, shown: bool) {
        self.chrome.legend = shown;
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

/// A clock for the widgets that draw something moving.
///
/// One type rather than an `Instant` threaded through each loop, because the only thing any of
/// them wants is the elapsed milliseconds and the only thing that can go wrong is forgetting to
/// start it.
pub(crate) struct Since(std::time::Instant);

impl Since {
    pub(crate) fn now() -> Since {
        Since(std::time::Instant::now())
    }

    pub(crate) fn ms(&self) -> u64 {
        self.0.elapsed().as_millis() as u64
    }
}

/// The next key, or the deadline for the next frame.
///
/// `tick` is `None` for a widget whose frame only changes when you touch it, and that case blocks
/// exactly as before — an animation that costs a wakeup on an idle prompt is worth having only
/// while there is something animating. With a tick, [`crate::term::Pressed::Timeout`]
/// means "nothing was typed, draw the next frame", which is a different answer from `Ended` and
/// has to stay that way.
pub(crate) fn awaited(keys: &mut crate::term::Keys, tick: Option<i32>) -> Pressed {
    match tick {
        Some(ms) => keys.read_within(ms),
        None => match keys.read() {
            Some(key) => Pressed::Key(key),
            None => Pressed::Ended,
        },
    }
}

/// `text` with the cursor drawn on the character at `at`.
///
/// The real terminal cursor is hidden for every widget — an inline one repaints its whole block on
/// each keystroke and the cursor is dragged across all of it — so a text field has to draw its own.
/// This is what bubbletea does for gum, and it is better than positioning the real one for a
/// reason beyond flicker: the caret is *part of the frame*, so it cannot end up a cell out of step
/// with the text the way a separate `ESC [ n C` can.
///
/// **The shape is the one the config asked for.** `oslo.vi.cursor_insert` names a `DECSCUSR` shape
/// that a real cursor would take, and a drawn one that ignored it left the shell with two
/// different cursors depending on which widget you were in. Since the terminal is not drawing this
/// one, each shape is emulated: a block reverses the cell, an underline underlines it, and a bar
/// is a thin rule down its left edge.
///
/// At the end of the text the caret sits on a space, which is how you can tell "typing at the end"
/// from "on the last character".
pub(crate) fn with_caret(text: &str, at: usize) -> String {
    with_caret_on(text, at, None)
}

/// [`with_caret`], on a surface.
///
/// **A caret drawn with no background punches a hole in the panel it sits on.** The row around it
/// carries the input surface; the caret carried the terminal's own colour, so on a tinted bar it
/// showed as a black cell in the middle of an otherwise continuous block — the one cell that did
/// not belong. The same rule as everything else painted on a surface: every cell of a coloured row
/// is painted, gaps included.
pub(crate) fn with_caret_on(text: &str, at: usize, surface: Option<crate::theme::Color>) -> String {
    // Insert, because a text field is where you insert. The blink variants are drawn steady: a
    // frame is redrawn on a keystroke, not on a timer, so a blinking caret would blink only while
    // you typed — which reads as a rendering fault rather than as a cursor.
    let shape = crate::settings::current().vi.cursors.insert;
    caret_over(text, at, shape, surface)
}

/// [`with_caret`] with the shape given rather than read.
///
/// Split out so the three shapes can be tested without a config: reading a global inside the thing
/// under test means only whichever shape happens to be the default is ever exercised.
fn caret_over(
    text: &str,
    at: usize,
    shape: crate::vi::Cursor,
    surface: Option<crate::theme::Color>,
) -> String {
    use crate::theme::Style;
    use crate::vi::Cursor;

    let depth = crate::theme::depth();
    let chars: Vec<char> = text.chars().collect();
    let at = at.min(chars.len());
    let before: String = chars[..at].iter().collect();
    let under = chars.get(at).copied().unwrap_or(' ');
    let after: String = chars.iter().skip(at + 1).collect();
    // Whatever the shape, it occupies exactly the one cell the character does. A caret that took a
    // second cell would shift the text under it every time it moved, and the row is measured in
    // cells everywhere else.
    let reversed = Style {
        reverse: true,
        ..Style::default()
    };
    let (marked, style) = match shape {
        // A real bar sits *between* cells and costs no width; drawn, it has to live in one. So it
        // is a bar wherever there is an empty cell to be one in — the end of a line, which is
        // where a caret spends nearly all its time — and reverses the character otherwise rather
        // than hiding it.
        Cursor::Bar | Cursor::BlinkBar if under == ' ' => {
            ("▏".to_string(), crate::theme::current().ui.accent)
        }
        Cursor::Underline | Cursor::BlinkUnderline => (
            under.to_string(),
            Style {
                underline: true,
                // Under a space an underline is the whole cursor, and a bare `SGR 4` on a blank
                // cell is invisible in some terminals. The accent gives it something to draw.
                ..crate::theme::current().ui.accent
            },
        ),
        _ => (under.to_string(), reversed),
    };
    // The surface goes under the caret and under the text either side of it, so the whole run is
    // one continuous block.
    //
    // **The reversal is done by hand when there is a surface.** `SGR 7` swaps against the
    // terminal's *default* background, not against the colour the row is painted on — so a caret
    // left to reverse itself came out as a hole of the default colour in the middle of a tinted
    // row, which is exactly as wrong as it sounds and reads as a rendering fault. Swapping the two
    // colours here keeps the mark and keeps it on the surface it is sitting on.
    let on = |style: Style| match (style.reverse, surface) {
        (true, Some(under)) => Style {
            reverse: false,
            fg: Some(under),
            bg: crate::theme::current().ui.accent.fg,
            ..Style::default()
        },
        (true, None) => style,
        (false, _) => Style {
            bg: surface.or(style.bg),
            ..style
        },
    };
    let plain = on(Style::default());
    format!(
        "{}{}{}",
        plain.paint(&before, depth),
        on(style).paint(&marked, depth),
        plain.paint(&after, depth)
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

    use crate::vi::Cursor;

    /// Whether any SGR in `drawn` sets attribute `code`.
    ///
    /// Matched as a parameter rather than as the literal `\x1b[4m`, because a shape carries the
    /// accent colour with it and the terminal is sent all of them at once: `\x1b[1;4;95m`.
    fn sets(drawn: &str, code: &str) -> bool {
        drawn.split('\x1b').any(|part| {
            part.strip_prefix('[')
                .and_then(|p| p.split_once('m'))
                .is_some_and(|(params, _)| params.split(';').any(|p| p == code))
        })
    }

    /// The caret is part of the text, so it can never be a cell out of step with it.
    #[test]
    fn the_caret_marks_the_character_it_is_on() {
        let _depth = crate::theme::held_at(crate::theme::Depth::Ansi256);
        // Reverse video around exactly one character, and the rest untouched.
        let drawn = caret_over("abc", 1, Cursor::Block, None);
        assert!(drawn.contains("\x1b[7m"), "not reversed: {drawn:?}");
        assert_eq!(plain(&drawn), "abc", "the text changed: {drawn:?}");
    }

    /// Past the end it sits on the cell after the text, which is how "typing at the end" looks
    /// different from "on the last character".
    #[test]
    fn at_the_end_the_caret_is_the_next_cell() {
        assert_eq!(plain(&caret_over("ab", 2, Cursor::Block, None)), "ab ");
        // And an index past even that cannot panic — a cursor arriving from shell code is a
        // number somebody could have written.
        assert_eq!(plain(&caret_over("ab", 99, Cursor::Block, None)), "ab ");
        assert_eq!(plain(&caret_over("", 0, Cursor::Block, None)), " ");
    }

    /// **The shape is the one the config asked for.** A widget drawing its own hard-coded block
    /// left the shell with two different cursors depending on which one you were in.
    #[test]
    fn the_caret_takes_the_configured_shape() {
        let _depth = crate::theme::held_at(crate::theme::Depth::Ansi256);
        let block = caret_over("ab", 0, Cursor::Block, None);
        assert!(sets(&block, "7"), "not reversed: {block:?}");

        let under = caret_over("ab", 0, Cursor::Underline, None);
        assert!(sets(&under, "4"), "not underlined: {under:?}");
        assert!(!sets(&under, "7"), "and not also reversed: {under:?}");
        assert_eq!(plain(&under), "ab", "the character is still readable");

        // A bar is a bar where there is an empty cell to be one in.
        assert_eq!(plain(&caret_over("ab", 2, Cursor::Bar, None)), "ab▏");
        // And reverses the character rather than hiding it where there is not.
        let over_text = caret_over("ab", 0, Cursor::Bar, None);
        assert_eq!(plain(&over_text), "ab", "{over_text:?}");
        assert!(sets(&over_text, "7"), "{over_text:?}");
    }

    /// Whatever the shape, it costs exactly one cell. A caret that took two would shift the text
    /// under it every time it moved, and every row here is measured in cells.
    #[test]
    fn every_shape_is_one_cell_wide() {
        use crate::prompt::printed_width;
        for shape in [
            Cursor::Block,
            Cursor::BlinkBlock,
            Cursor::Underline,
            Cursor::BlinkUnderline,
            Cursor::Bar,
            Cursor::BlinkBar,
        ] {
            for (text, at, want) in [("abc", 1, 3), ("abc", 3, 4), ("", 0, 1)] {
                let drawn = caret_over(text, at, shape, None);
                assert_eq!(printed_width(&drawn), want, "{shape:?} on {text:?}@{at}");
            }
        }
    }

    /// A multibyte character is one cell, not one byte — slicing by byte would panic on it.
    #[test]
    fn the_caret_counts_characters_not_bytes() {
        assert_eq!(plain(&caret_over("héllo", 1, Cursor::Block, None)), "héllo");
        assert_eq!(plain(&caret_over("→x", 0, Cursor::Block, None)), "→x");
    }
}

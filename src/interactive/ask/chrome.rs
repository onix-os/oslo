//! What a widget is wrapped in: the legend, a border, the whole screen, and where on it.
//!
//! Every widget in this module draws a *frame* — rows joined by `\r\n` — and hands it to
//! `super::Inline`. This sits between: it takes that frame and decides what surrounds it.
//!
//! Four questions, and they compose:
//!
//! * **legend** — the `↑↓ move • enter choose` row. On by default, because a prompt whose keys you
//!   have to guess is one you leave by closing the window. Off for a widget embedded in something
//!   that explains itself.
//! * **border** — a box, either hugging the content or drawn to the full width of the terminal.
//!   Content-width is right for a prompt in a transcript; full-width is right for a heading, and is
//!   the only one that looks deliberate on a screen with nothing else on it.
//! * **fullscreen** — the alternate screen, which a terminal keeps separate from the scrollback.
//!   Entering and leaving are one escape each, and leaving restores the transcript exactly, which
//!   is the whole reason to use it rather than clearing.
//! * **alignment** — only meaningful once there is a screen to be somewhere on. Top-left is where
//!   everything already was; centring is what makes a small widget on a big screen look placed
//!   rather than stranded.
//!
//! # Why this is not five copies
//!
//! `footer` already exists because each widget appending its own keys drifted — `confirm` had the
//! question, the buttons and the keys on one line while its siblings had three. This is the same
//! argument one level out: a border that hugs in one widget and hugs-plus-one in another is not a
//! style, it is a bug nobody can name.

use super::style::Border;
use crate::interactive::dropdown::width;
use crate::interactive::theme::{self, Style};

/// How wide a border is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Fit {
    /// As wide as the widest row inside it.
    #[default]
    Content,
    /// The full width of the terminal.
    Full,
}

impl Fit {
    pub fn parse(name: &str) -> Option<Fit> {
        Some(match name.trim() {
            "content" | "fit" | "hug" => Fit::Content,
            "full" | "terminal" | "wide" => Fit::Full,
            _ => return None,
        })
    }
}

/// Where the frame sits. Only meaningful in [`Chrome::fullscreen`] for the vertical axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Place {
    #[default]
    Start,
    Center,
    End,
}

impl Place {
    /// `left`/`top` and `right`/`bottom` are the same two ends under different names, so one parser
    /// answers for both axes and a caller cannot write `align_y = "left"` and be quietly obeyed.
    pub fn parse(name: &str) -> Option<Place> {
        Some(match name.trim() {
            "start" | "left" | "top" => Place::Start,
            "center" | "centre" | "middle" => Place::Center,
            "end" | "right" | "bottom" => Place::End,
            _ => return None,
        })
    }

    /// How many cells of padding go before the content, given the room to spare.
    fn offset(self, slack: usize) -> usize {
        match self {
            Place::Start => 0,
            Place::Center => slack / 2,
            Place::End => slack,
        }
    }
}

/// Everything that surrounds a widget's own rows.
#[derive(Debug, Clone)]
pub struct Chrome {
    /// Whether to draw the key legend. See the module note.
    pub legend: bool,
    pub border: Border,
    pub border_style: Style,
    pub fit: Fit,
    /// Draw on the alternate screen, which the terminal keeps out of the scrollback.
    pub fullscreen: bool,
    pub align_x: Place,
    pub align_y: Place,
}

impl Default for Chrome {
    fn default() -> Chrome {
        Chrome {
            // On, because that is what every widget did before there was a choice, and the row it
            // costs is cheaper than a prompt nobody can leave.
            legend: true,
            border: Border::None,
            border_style: Style::default(),
            fit: Fit::Content,
            fullscreen: false,
            align_x: Place::Start,
            align_y: Place::Start,
        }
    }
}

impl Chrome {
    /// Whether anything here changes how the frame is drawn.
    ///
    /// The common case is "no", and it is worth answering cheaply: a widget with default chrome
    /// must cost exactly what it cost before this module existed.
    pub fn is_plain(&self) -> bool {
        !self.fullscreen && self.border == Border::None && self.align_x == Place::Start
    }

    /// Wrap `frame` in whatever this asks for.
    ///
    /// Rows are separated by `\r\n`, and each carries its own `\r\x1b[K` so a redraw erases what it
    /// replaces. Both are rebuilt here rather than preserved, because a bordered row is a different
    /// row: it starts at the border, not at the content.
    pub fn wrap(&self, frame: &str) -> String {
        let rows = rows_of(frame);
        let rows = match self.border {
            Border::None => rows,
            border => self.boxed(&rows, border),
        };
        let rows = self.indent(rows);
        rows.iter()
            .map(|row| format!("\r\x1b[K{row}"))
            .collect::<Vec<_>>()
            .join("\r\n")
    }

    /// The rows a bordered frame has.
    fn boxed(&self, rows: &[String], border: Border) -> Vec<String> {
        let Some(glyphs) = border.glyphs() else {
            return rows.to_vec();
        };
        let [
            top_left,
            top_right,
            bottom_left,
            bottom_right,
            horizontal,
            vertical,
        ] = glyphs;
        let depth = theme::depth();
        let paint = |s: &str| self.border_style.paint(s, depth);

        // The inside width: the widest row, or the terminal minus the two border columns.
        let widest = rows.iter().map(|r| printed(r)).max().unwrap_or(0);
        let inside = match self.fit {
            Fit::Content => widest,
            Fit::Full => width::terminal_cols().saturating_sub(2).max(widest.min(1)),
        };

        let rule = horizontal.repeat(inside);
        let mut out = Vec::with_capacity(rows.len() + 2);
        out.push(paint(&format!("{top_left}{rule}{top_right}")));
        for row in rows {
            // Padded to the inside width so the right-hand edge is straight, measured in printed
            // cells rather than bytes — a coloured row is mostly escape sequences.
            let pad = inside.saturating_sub(printed(row));
            out.push(format!(
                "{}{row}{}{}",
                paint(vertical),
                " ".repeat(pad),
                paint(vertical)
            ));
        }
        out.push(paint(&format!("{bottom_left}{rule}{bottom_right}")));
        out
    }

    /// Move the whole frame right, if it was asked to sit somewhere other than the left edge.
    fn indent(&self, rows: Vec<String>) -> Vec<String> {
        if self.align_x == Place::Start {
            return rows;
        }
        let widest = rows.iter().map(|r| printed(r)).max().unwrap_or(0);
        let slack = width::terminal_cols().saturating_sub(widest);
        let lead = " ".repeat(self.align_x.offset(slack));
        rows.into_iter().map(|row| format!("{lead}{row}")).collect()
    }

    /// The blank rows that go above the frame to place it vertically.
    ///
    /// Only on the alternate screen: inline, the widget is already at the cursor and pushing it
    /// down would just scroll the transcript.
    pub fn top_margin(&self, frame_rows: usize) -> usize {
        if !self.fullscreen || self.align_y == Place::Start {
            return 0;
        }
        let slack = width::terminal_rows().saturating_sub(frame_rows);
        self.align_y.offset(slack)
    }
}

/// The rows of a frame, with the redraw escapes stripped.
fn rows_of(frame: &str) -> Vec<String> {
    frame
        .split("\r\n")
        .map(|row| {
            let row = row.trim_start_matches('\r');
            row.strip_prefix("\x1b[K").unwrap_or(row).to_string()
        })
        .collect()
}

/// A row's width in cells, ignoring colour.
fn printed(row: &str) -> usize {
    crate::interactive::prompt::printed_width(row)
}

/// Enter the alternate screen: a second buffer the terminal keeps out of the scrollback.
///
/// `?1049h` saves the cursor and switches; `?1049l` switches back and restores it, which is what
/// makes leaving put the transcript back exactly rather than approximately. `?25l` hides the
/// cursor, because a widget draws its own.
pub fn enter_fullscreen() {
    super::show("\x1b[?1049h\x1b[?25l\x1b[H");
}

/// Leave it, putting back everything that was on screen before.
pub fn leave_fullscreen() {
    super::show("\x1b[?25h\x1b[?1049l");
}

#[cfg(test)]
mod tests;

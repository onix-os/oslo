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
    /// Blank rows between the content and the legend's rule.
    ///
    /// One by default. Run together, the thing you are answering and the note *about* the widget
    /// read as one block and the eye has to work out which part is which.
    pub legend_gap: usize,
    /// Blank columns inside the border, each side.
    pub padding_x: usize,
    /// Blank rows inside the border, top and bottom.
    pub padding_y: usize,
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
            legend_gap: 1,
            // A box whose text touches its own walls looks like a mistake, and one cell is what
            // every other box-drawing tool settles on. Vertical padding is off: a widget is
            // already several rows tall and two more is a lot of screen for a prompt.
            padding_x: 1,
            padding_y: 0,
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

    /// Cells of padding inside the border, each side. Only with a border — without one it would be
    /// an indent, which `align_x` already is.
    fn pad_x(&self) -> usize {
        match self.border {
            Border::None => 0,
            _ => self.padding_x,
        }
    }

    fn pad_y(&self) -> usize {
        match self.border {
            Border::None => 0,
            _ => self.padding_y,
        }
    }

    /// How many rows the legend block adds: the gap, the rule and the keys.
    ///
    /// Zero with the legend off, so turning it off gives a list those rows back rather than leaving
    /// a hole where they were. Widgets size their window against this.
    pub fn legend_rows(&self) -> usize {
        match self.legend {
            false => 0,
            true => self.legend_gap + 2,
        }
    }

    /// Every row the chrome adds around a frame, for a caller sizing its window.
    pub fn extra_rows(&self) -> usize {
        let border = match self.border {
            Border::None => 0,
            _ => 2,
        };
        self.legend_rows() + border + self.pad_y() * 2
    }

    /// Wrap `frame` in whatever this asks for, and put `keys` under it.
    ///
    /// **The legend is built here rather than by the widget**, and that is what makes its rule the
    /// right width: the rule has to span the finished box, which is not known until the legend's
    /// own width has been counted. A widget appending its own footer could only measure the content
    /// above it, so inside a border the rule came out a fifth of the width and read as damage.
    pub fn wrap(&self, frame: &str, keys: &[(&str, &str)]) -> String {
        // A frame begins with `\r\n` — a widget draws on the row *below* the prompt — so the first
        // split is the caller's own row rather than content. Treating it as a row is what put a
        // blank line inside the top of every box.
        let lead = frame.starts_with("\r\n");
        let mut rows = rows_of(frame);
        if lead && rows.first().is_some_and(|first| first.is_empty()) {
            rows.remove(0);
        }

        let legend = self.legend && !keys.is_empty();
        if legend {
            rows.extend(std::iter::repeat_n(String::new(), self.legend_gap));
            // A placeholder: the rule cannot be sized until the legend under it has been measured.
            rows.push(String::new());
            rows.push(legend_text(keys));
        }

        let inside = self.inside_width(&rows);
        if legend {
            let at = rows.len() - 2;
            rows[at] = self.rule(inside);
        }

        let rows = self.padded(rows, inside);
        let rows = match self.border {
            Border::None => rows,
            border => self.boxed(&rows, border, inside + self.pad_x() * 2),
        };
        let rows = self.indent(rows);

        let body = rows
            .iter()
            .map(|row| format!("\r\x1b[K{row}"))
            .collect::<Vec<_>>()
            .join("\r\n");
        match lead {
            true => format!("\r\n{body}"),
            false => body,
        }
    }

    /// The width the content is laid out to, before padding and border.
    fn inside_width(&self, rows: &[String]) -> usize {
        let widest = rows.iter().map(|r| printed(r)).max().unwrap_or(0);
        match self.fit {
            Fit::Content => widest,
            // The terminal, less the two border columns and the padding each side. A `Full` box
            // that ignored its own padding would run off the right-hand edge by exactly that much.
            Fit::Full => width::terminal_cols()
                .saturating_sub(2 + self.pad_x() * 2)
                .max(widest),
        }
    }

    /// The tear-off rule above the legend, exactly as wide as the space it sits in.
    ///
    /// `- ` rather than `─`, so it reads as a tear-off rather than as a second border. Cut to the
    /// width and any trailing space dropped: turning that space into a dash gave `- - - --` on an
    /// even width, which reads as a typo.
    fn rule(&self, inside: usize) -> String {
        let pattern: String = "- "
            .repeat(inside.div_ceil(2))
            .chars()
            .take(inside)
            .collect();
        let muted = theme::current().ui.muted;
        muted.paint(pattern.trim_end(), theme::depth())
    }

    /// Every row to the same width, plus the blank rows above and below.
    ///
    /// Done here rather than inside `boxed` so that an *unbordered* frame is squared off too — a
    /// block that is not rectangular shears the moment `align_x` moves it.
    fn padded(&self, rows: Vec<String>, inside: usize) -> Vec<String> {
        let side = " ".repeat(self.pad_x());
        let blank = format!("{side}{}{side}", " ".repeat(inside));
        let mut out = Vec::with_capacity(rows.len() + self.pad_y() * 2);
        out.extend(std::iter::repeat_n(blank.clone(), self.pad_y()));
        for row in rows {
            // Measured in printed cells, not bytes: a coloured row is mostly escape sequences, and
            // padding by byte length is how a box comes out ragged.
            let fill = inside.saturating_sub(printed(&row));
            out.push(format!("{side}{row}{}{side}", " ".repeat(fill)));
        }
        out.extend(std::iter::repeat_n(blank, self.pad_y()));
        out
    }

    /// The rows a bordered frame has. `width` is the inside, padding included.
    fn boxed(&self, rows: &[String], border: Border, width: usize) -> Vec<String> {
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

        let rule = horizontal.repeat(width);
        let mut out = Vec::with_capacity(rows.len() + 2);
        out.push(paint(&format!("{top_left}{rule}{top_right}")));
        for row in rows {
            out.push(format!("{}{row}{}", paint(vertical), paint(vertical)));
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

/// `key what` pairs joined by ` • `, dim.
///
/// Lives here rather than in `super` because the rule above it has to be sized against it, and the
/// two were in different files for exactly as long as the rule was the wrong width.
pub(super) fn legend_text(keys: &[(&str, &str)]) -> String {
    let ui = theme::current().ui;
    let depth = theme::depth();
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

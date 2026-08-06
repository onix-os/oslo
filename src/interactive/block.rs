//! A headline and a rail of labelled rows: the shape every report the shell prints already has.
//!
//! ```text
//! direnv ~/data/code/tools/rush
//!   │ changed  PATH
//!   │ aliases  _b _c _r _t _v
//! ```
//!
//! # Why this is here rather than in the reporter that needed it
//!
//! Five subsystems print a block like this — the directory environment, the job notice, the
//! slow-command notice, `chain`, and `time` — and each grew its own formatter. `oslo.ui.block`
//! gives a config the same shape, and **a Lua API that is a second implementation of what the
//! shell does internally is one that drifts**: it is always missing whatever the built-in renderer
//! learned last, and nobody trusts it. So the drawing lives here, once, and both the reporters and
//! the Lua binding are callers.
//!
//! # Overflow is the caller's decision
//!
//! A row that does not fit has to do something, and which something depends on what the row *is*:
//!
//! * `Overflow::Count` — cut, and say how many were dropped (` +12`). Right for a list of names,
//!   where the count is information and the thirteenth name is not. A Nix dev shell changes
//!   thirty-five variables, and printed in full that is four wrapped lines on every `cd`.
//! * `Overflow::Ellipsis` — cut, and end with `…`. Right for one long value, where the beginning
//!   is the interesting part.
//! * `Overflow::Wrap` — continue on the next line, keeping the rail. Right when the content has
//!   to be read rather than skimmed.
//!
//! Before this existed the choice was `Count`, hardcoded, everywhere.
//!
//! # Rendered, not printed
//!
//! `Block::lines` gives the rows back; nothing here writes to a terminal. That is what lets the
//! whole block be emitted in one `print!` — a block assembled across several statements must not
//! interleave with a command's own output — and what lets it be tested without a tty.

use crate::interactive::dropdown::width;
use crate::interactive::theme::{self, Color, Style};

/// The rail glyph and the indent before it. Two cells of indent, so a block sits under its
/// headline rather than against the left margin.
const INDENT: &str = "  ";
const RAIL: &str = "│";

/// How wide a label column is padded to, so names start in the same column on every row.
///
/// A ragged left edge is the difference between a list you can scan and one you have to read.
const LABEL_WIDTH: usize = 7;

/// What a row does when it does not fit the terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Overflow {
    /// Cut, and append ` +N` for what was dropped. Counts whitespace-separated items.
    #[default]
    Count,
    /// Cut, and append `…`.
    Ellipsis,
    /// Continue on the next line, indented under the first and keeping the rail.
    Wrap,
}

impl Overflow {
    /// The spelling a Lua caller writes. Unknown names answer `None` so the binding can refuse
    /// them rather than silently picking a default the caller did not ask for.
    pub fn named(name: &str) -> Option<Overflow> {
        match name.trim() {
            "count" => Some(Overflow::Count),
            "ellipsis" => Some(Overflow::Ellipsis),
            "wrap" => Some(Overflow::Wrap),
            _ => None,
        }
    }
}

/// One rail row.
struct Row {
    label: String,
    label_style: Style,
    text: String,
    text_style: Style,
    overflow: Overflow,
}

/// A headline and its rows, built up and then rendered.
pub struct Block {
    head: String,
    rows: Vec<Row>,
    /// Whether to draw the rail and paint at all. Off when nothing is watching — a block in a pipe
    /// is plain text, because a rail glyph in somebody's `grep` is not a decoration, it is damage.
    decorated: bool,
    /// The width to fit to. Taken once at construction so every row of one block agrees, even if
    /// the terminal is resized while it is being built.
    columns: usize,
}

impl Block {
    /// A block headed by `head`, drawn for a terminal.
    pub fn new(head: impl Into<String>) -> Block {
        Block {
            head: head.into(),
            rows: Vec::new(),
            decorated: true,
            columns: width::terminal_cols(),
        }
    }

    /// Draw plain: no rail, no colour. For a block whose output is not going to a terminal.
    pub fn plain(mut self) -> Block {
        self.decorated = false;
        self
    }

    /// Fit to `columns` rather than to the terminal. For tests, and for a caller drawing into a
    /// space it owns.
    pub fn width(mut self, columns: usize) -> Block {
        self.columns = columns.max(1);
        self
    }

    /// A labelled row.
    pub fn row(&mut self, label: impl Into<String>, text: impl Into<String>) -> &mut Block {
        self.push(
            label.into(),
            Style::default(),
            text.into(),
            Style::default(),
        )
    }

    /// A labelled row, with the label and the text painted.
    pub fn styled_row(
        &mut self,
        label: impl Into<String>,
        label_style: Style,
        text: impl Into<String>,
        text_style: Style,
    ) -> &mut Block {
        self.push(label.into(), label_style, text.into(), text_style)
    }

    /// A row with no label — a detail line under whatever came before it.
    pub fn note(&mut self, text: impl Into<String>) -> &mut Block {
        self.push(String::new(), Style::default(), text.into(), dim())
    }

    /// How a row overflows. Applies to the row just added.
    pub fn overflow(&mut self, how: Overflow) -> &mut Block {
        if let Some(row) = self.rows.last_mut() {
            row.overflow = how;
        }
        self
    }

    fn push(
        &mut self,
        label: String,
        label_style: Style,
        text: String,
        text_style: Style,
    ) -> &mut Block {
        self.rows.push(Row {
            label,
            label_style,
            text,
            text_style,
            overflow: Overflow::default(),
        });
        self
    }

    /// Whether anything but the headline was added.
    pub fn is_bare(&self) -> bool {
        self.rows.is_empty()
    }

    /// The block as lines, headline first. Nothing is written; see the module note.
    pub fn lines(&self) -> Vec<String> {
        let mut out = Vec::with_capacity(self.rows.len() + 1);
        if !self.head.is_empty() {
            out.push(self.head.clone());
        }
        for row in &self.rows {
            self.render_row(row, &mut out);
        }
        out
    }

    /// The fixed cells a row spends before its text: indent, rail, a space, the label column, a
    /// space. Measured rather than assumed, so a caller that changes the indent cannot silently
    /// make every row one cell too wide.
    fn prefix_width(&self) -> usize {
        if !self.decorated {
            return INDENT.len() + LABEL_WIDTH + 1;
        }
        INDENT.len() + width::display_width(RAIL) + 1 + LABEL_WIDTH + 1
    }

    fn render_row(&self, row: &Row, out: &mut Vec<String>) {
        let budget = self.columns.saturating_sub(self.prefix_width()).max(1);
        let label = format!("{:<LABEL_WIDTH$}", row.label);
        let lead = self.lead(&paint(&label, row.label_style, self.decorated));

        match row.overflow {
            Overflow::Wrap => {
                let mut first = true;
                for piece in wrap(&row.text, budget) {
                    let painted = paint(&piece, row.text_style, self.decorated);
                    if first {
                        out.push(format!("{lead} {painted}"));
                        first = false;
                    } else {
                        // Continuation rows keep the rail and blank the label, so the text stays
                        // in one column and the block still reads as one thing.
                        let blank = self.lead(&" ".repeat(LABEL_WIDTH));
                        out.push(format!("{blank} {painted}"));
                    }
                }
                if first {
                    out.push(format!("{lead} "));
                }
            }
            Overflow::Ellipsis => {
                let (shown, cut) = cut_to(&row.text, budget);
                let mut line = format!("{lead} {}", paint(&shown, row.text_style, self.decorated));
                if cut {
                    line.push_str(&paint("…", dim(), self.decorated));
                }
                out.push(line);
            }
            Overflow::Count => {
                let (shown, hidden) = fit_items(&row.text, budget);
                let mut line = format!("{lead} {}", paint(&shown, row.text_style, self.decorated));
                if hidden > 0 {
                    line.push_str(&paint(&format!(" +{hidden}"), dim(), self.decorated));
                }
                out.push(line);
            }
        }
    }

    /// Everything left of a row's text: the indent, the rail, and the label column.
    fn lead(&self, label: &str) -> String {
        if self.decorated {
            format!("{INDENT}{} {label}", paint(RAIL, dim(), true))
        } else {
            format!("{INDENT}{label}")
        }
    }
}

fn dim() -> Style {
    Style::fg(Color::Indexed(240))
}

fn paint(text: &str, style: Style, decorated: bool) -> String {
    if decorated {
        style.paint(text, theme::depth())
    } else {
        text.to_string()
    }
}

/// As many whitespace-separated items of `text` as fit in `budget`, and how many did not.
fn fit_items(text: &str, budget: usize) -> (String, usize) {
    let items: Vec<&str> = text.split_whitespace().collect();
    let mut out = String::new();
    let mut used = 0usize;
    let mut shown = 0usize;
    for item in &items {
        let w = width::display_width(item) + usize::from(shown > 0);
        // At least one item always goes in, however narrow the terminal: a row that showed nothing
        // and said `+35` would be worse than one that overflows by a few cells.
        if used + w > budget && shown > 0 {
            break;
        }
        if shown > 0 {
            out.push(' ');
        }
        out.push_str(item);
        used += w;
        shown += 1;
    }
    (out, items.len().saturating_sub(shown))
}

/// `text` cut to `budget` cells, and whether anything was cut.
///
/// One cell is kept back for the `…` when there is something to cut, so the result never overflows
/// the budget it was given.
fn cut_to(text: &str, budget: usize) -> (String, bool) {
    if width::display_width(text) <= budget {
        return (text.to_string(), false);
    }
    let room = budget.saturating_sub(1);
    let mut out = String::new();
    let mut used = 0usize;
    for ch in text.chars() {
        let w = width::display_width(&ch.to_string());
        if used + w > room {
            break;
        }
        out.push(ch);
        used += w;
    }
    (out, true)
}

/// `text` broken into pieces of at most `budget` cells, on whitespace where it can be.
fn wrap(text: &str, budget: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    let mut used = 0usize;
    for word in text.split_whitespace() {
        let w = width::display_width(word);
        let with_space = w + usize::from(used > 0);
        if used + with_space > budget && used > 0 {
            lines.push(std::mem::take(&mut line));
            used = 0;
        }
        // A single word longer than the whole budget is broken rather than left to wrap the
        // terminal itself, which would corrupt a redraw rather than merely look untidy.
        if w > budget {
            for piece in hard_break(word, budget) {
                if !line.is_empty() {
                    lines.push(std::mem::take(&mut line));
                }
                lines.push(piece);
            }
            used = 0;
            continue;
        }
        if used > 0 {
            line.push(' ');
            used += 1;
        }
        line.push_str(word);
        used += w;
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

fn hard_break(word: &str, budget: usize) -> Vec<String> {
    let mut pieces = Vec::new();
    let mut piece = String::new();
    let mut used = 0usize;
    for ch in word.chars() {
        let w = width::display_width(&ch.to_string());
        if used + w > budget && used > 0 {
            pieces.push(std::mem::take(&mut piece));
            used = 0;
        }
        piece.push(ch);
        used += w;
    }
    if !piece.is_empty() {
        pieces.push(piece);
    }
    pieces
}

#[cfg(test)]
mod tests;

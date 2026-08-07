//! Drawing the finder: one full-screen frame per keystroke.
//!
//! # Why a whole frame every time
//!
//! The finder owns the alternate screen while it is open, so there is no prompt underneath to
//! preserve and no scrollback to protect — which is exactly what the completion dropdown has to
//! work around, and why that code is the shape it is. Here the cheap correct thing is available:
//! move home, draw every row, erase to the end of each. No diffing, no cursor arithmetic, nothing
//! to get one row out of step with.
//!
//! # The layout
//!
//! ```text
//!     2h    3× git status
//!     5h   41× cargo build --release
//!   ❯ 1d  118× cargo test                                         ← selected
//!                                                                 ┐
//!   ⬝⬝⬝⬝⬝⬝⬝⬝⬝  ❯❯  car▏           default @ [global] || 12/840    ├ the surface
//!                                                                 ┘
//! ```
//!
//! **The list grows upward from the search bar.** The bar is at the bottom because that is where
//! the cursor is and where your eyes already are; the first result sits nearest it, across one
//! separating row, so the likely choice stays close without merging into the input. fzf and atuin
//! both settled on bottom-up results, and the reason is the same one.
//!
//! # None of that is drawn here
//!
//! Every row above comes from [`crate::ui::ask::look`], and specifically from
//! [`Preset::History`] — the striping, the metadata columns, the match marks, the three-row
//! surface, the sweep, the badge and the counter. This module decides *what* the rows say and
//! where on the screen the block goes; the look decides what they look like.
//!
//! That is a swap, not a refactor for its own sake. The finder owned four hundred lines of row
//! painting, and when `ui filter` grew the same abilities it grew a second nearly identical copy —
//! two renderers that had to be kept in step by hand and immediately were not. Now a fix to the
//! bar arithmetic or the stripe lands in both, and the preset is checkable: if `--look history`
//! stops looking like the finder, a test says so.
//!
//! The one thing still drawn here is the delete confirmation, because it is not a list: it takes
//! over the bar's three rows to ask a question.

use super::Scope;
use super::rank::{Ranked, ago};
use crate::ui::ask::look::{Look, Preset, Row, View};
use crate::ui::paint::{SYNC_BEGIN, SYNC_END};
use crate::ui::prompt::printed_width;
use crate::ui::theme::{self, Color, Depth, Style};

/// Rows the input surface takes: a blank row, the query, a blank row.
///
/// The blank rows are the surface, not spacing around it — they carry the same colour, which is
/// what makes the input read as a panel rather than as one coloured line. Three is codex's shape
/// and the smallest number that reads as deliberate: one row looks like a highlight, three looks
/// like somewhere to type.
const SURFACE_ROWS: usize = 3;

/// Unpainted rows below the surface, so the panel does not sit flush against the bottom edge.
const BOTTOM_MARGIN: usize = 1;

/// Unpainted row between the result list and the input surface.
const TOP_MARGIN: usize = 1;

/// Unpainted row at the very top of the screen, matching [`BOTTOM_MARGIN`] at the other end.
///
/// The panel had air beneath it and none above, so the list ran flush into the top edge while the
/// input floated — which reads as the whole thing having slipped upward rather than as a margin.
const SCREEN_MARGIN: usize = 1;

/// Everything the list does not get.
const CHROME_ROWS: usize = SCREEN_MARGIN + TOP_MARGIN + SURFACE_ROWS + BOTTOM_MARGIN;

/// What the frame needs to know about the world.
pub struct Frame<'a> {
    pub matches: &'a [Ranked],
    pub selected: usize,
    /// The first visible row, so a long list can scroll under a fixed window.
    pub offset: usize,
    pub query: &'a str,
    /// How long the finder has been open, for the scanner in the search bar.
    pub elapsed_ms: u64,
    /// Which profile's history is being shown.
    pub profile: &'a str,
    /// When Delete is waiting to be confirmed, which button is selected.
    ///
    /// `Some(true)` is *yes*. The search bar becomes the question — the three rows it already
    /// owns are exactly the height of a bordered box, so nothing moves and the list keeps its
    /// place while you answer.
    pub confirm: Option<bool>,
    pub scope: Scope,
    /// How many commands there are in total, for the `12/840` counter.
    pub total: usize,
    pub cols: usize,
    pub rows: usize,
    /// Unix seconds, for the age column. Passed in so the frame is a pure function of its input
    /// and can be tested without a clock.
    pub now: i64,
}

impl Frame<'_> {
    /// How many list rows fit.
    pub fn visible_rows(&self) -> usize {
        visible_rows(self.rows)
    }
}

/// How many rows remain for results after the input and its margins.
pub(super) fn visible_rows(rows: usize) -> usize {
    rows.saturating_sub(CHROME_ROWS).max(1)
}

/// The look this screen is drawn with.
///
/// **`Preset::History` is this screen**, which is the point of it existing: the finder used to
/// own four hundred lines of row painting that `ui filter` then grew a second, nearly identical
/// copy of. Now there is one renderer and the finder is a caller of it — so a fix to the striping,
/// the match marks or the bar arithmetic lands in both, and neither can drift.
///
/// Only the two things the preset cannot know are set here: which profile is being searched, and
/// which scope the badge shows.
fn look_of(f: &Frame<'_>) -> Look {
    let mut look = Preset::History.look();
    // `profile @ [scope] || 3/57`, all on the right, because all three are facts about what you
    // are looking at rather than part of what you are typing. The profile goes in as a literal:
    // `Look::fill` leaves anything it does not recognise alone, and the profile is not one of the
    // fields a list knows about itself.
    look.right = format!("{} @ {{badge}} || {{n}}/{{total}} ", f.profile);
    look.badge = f.scope.label().to_string();
    look
}

/// The whole screen, as one string of escapes.
pub fn frame(f: &Frame<'_>) -> String {
    let depth = theme::depth();
    let look = look_of(f);
    let visible = f.visible_rows();

    let rows: Vec<Row> = f
        .matches
        .iter()
        .map(|row| Row {
            // When and how often come first, then the command: the two numbers are short and
            // fixed width, so leading with them gives the eye a ruler down the left of the screen.
            meta: vec![
                ago(f.now, row.command.last_at),
                format!("{}×", row.command.runs),
            ],
            ..Row::new(row.command.line.clone())
        })
        .collect();
    let view = View {
        selected: f.selected,
        offset: f.offset,
        height: visible,
        query: f.query,
        matched: f.matches.len(),
        total: f.total,
        marked: 0,
        cols: f.cols,
        // While the question is up the bar is the question, so the look does not draw one.
        filtering: f.confirm.is_none(),
        elapsed_ms: f.elapsed_ms,
    };

    let mut body = look.rows(&rows, &view);
    if let Some(yes) = f.confirm {
        // Asking: the surface becomes a bordered box, so it reads as a thing that wants an answer
        // rather than as the search bar with different words in it. It takes exactly the rows the
        // bar would have, so the list above does not shift while you decide and the row you are
        // about to delete stays under your eye.
        let pager = &theme::current().pager;
        body.extend(std::iter::repeat_n(String::new(), look.gap));
        body.extend((0..SURFACE_ROWS).map(|row| confirm_row(row, yes, pager, f.cols, depth)));
    }

    let mut out = String::from(SYNC_BEGIN);
    // Home, then draw downward. Every row erases to the end of the line as it goes, so a shorter
    // row cannot leave the tail of a longer one behind it.
    out.push_str("\x1b[H");
    // One untouched row against the top edge, so the panel has as much air above it as
    // `BOTTOM_MARGIN` gives it below. Drawn rather than skipped, because the alternate screen is
    // not guaranteed to be blank.
    for _ in 0..SCREEN_MARGIN {
        out.push_str("\x1b[2K\r\n");
    }
    for row in &body {
        out.push_str("\x1b[2K");
        out.push_str(row);
        out.push_str("\r\n");
    }
    // And a plain row under it, so the panel does not sit on the terminal edge.
    out.push_str("\x1b[2K");
    out.push_str(SYNC_END);
    out
}

/// One row of the delete confirmation.
///
/// **A border, not a fill.** The search bar is a filled panel because it is where you type; this
/// is a question, and a box that has been drawn *around* something is the shape every terminal
/// program uses to say "answer me". Reusing the same three rows means the list above does not
/// shift while you decide, so the row you are about to delete stays under your eye.
fn confirm_row(row: usize, yes: bool, pager: &theme::Pager, cols: usize, depth: Depth) -> String {
    let edge = pager.match_;
    let inner = cols.saturating_sub(2);
    match row {
        0 => edge.paint(&format!("╭{}╮", "─".repeat(inner)), depth),
        2 => edge.paint(&format!("╰{}╯", "─".repeat(inner)), depth),
        _ => {
            let question = "delete from history?";
            let (yes_label, no_label) = ("[ yes ]", "[ no ]");
            // The selected button is filled, the other one is not: one difference, and it is the
            // one being asked about. A colour change alone reads as decoration.
            let picked = Style {
                fg: Some(Color::Indexed(0)),
                bg: Some(Color::Indexed(1)),
                ..Style::default()
            };
            let plain = pager.text;

            // **Centred.** The question and its two answers are one object, so the whole run is
            // measured and the leftover split either side — padding only the right would leave it
            // sitting against the border it is supposed to be inside.
            let body = printed_width(question)
                + 2
                + printed_width(yes_label)
                + 2
                + printed_width(no_label);
            let left = inner.saturating_sub(body) / 2;
            let right = inner.saturating_sub(body + left);

            let tail = plain.paint(&" ".repeat(right), depth);
            let side = edge.paint("│", depth);
            format!(
                "{}{}{}{}{}{}{}{}{}",
                side,
                plain.paint(&" ".repeat(left), depth),
                plain.paint(question, depth),
                plain.paint("  ", depth),
                if yes { picked } else { plain }.paint(yes_label, depth),
                plain.paint("  ", depth),
                if yes { plain } else { picked }.paint(no_label, depth),
                tail,
                side,
            )
        }
    }
}

#[cfg(test)]
#[path = "render/tests.rs"]
mod tests;

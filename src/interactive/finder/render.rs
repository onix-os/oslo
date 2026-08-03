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
//!    git status                              3×    2h   ~/src/oslo
//!    cargo build --release                  41×    5h   ~/src/oslo
//!  ❯ cargo test                            118×    1d   ~/src/oslo   ← selected
//!                                                                    ┐
//!    ❯ car                                             12/840        ├ the surface, three rows
//!                                                                    ┘
//! ```
//!
//! **The list grows upward from the search bar.** The bar is at the bottom because that is where
//! the cursor is and where your eyes already are; the first result sits directly above it, so the
//! thing you are most likely to take is the thing nearest what you are typing. fzf and atuin both
//! settled here, and the reason is the same one.
//!
//! # Nothing is painted except the input
//!
//! The list rows carry **no background at all** — they are text on the terminal's own background,
//! and the selected one is marked by a `❯` and its weight rather than by a slab of colour. Only
//! the input is a surface, and it is three rows tall: a blank row, the query, a blank row, all
//! sharing one tint.
//!
//! This is codex's treatment and it is worth saying why it is better than the completion
//! dropdown's, which paints every row. A menu that appears *under a prompt* needs a background,
//! because the background is the only thing saying where the menu starts and where the shell's
//! output stops. A full-screen finder has no such problem — the whole screen is already its own —
//! so painting the rows spends the strongest signal available on information nobody needed, and
//! leaves nothing to mark the thing that matters. Here the tint means "this is where you are
//! typing", and it is the only tinted thing on screen.
//!
//! The colour is the dropdown's own — `oslo.theme.pager.bg` — so the two surfaces in the shell
//! match and a theme sets them both at once. codex derives its tint from the terminal's live
//! background instead, which is a nicer idea and a worse fit here: oslo already has a themed
//! colour for exactly this, and deriving a second one would mean the finder ignored the theme the
//! user configured.

use super::rank::{Ranked, ago};
use crate::interactive::dropdown::width::{pad_to_width, truncate_to_width};
use crate::interactive::prompt::printed_width;
use crate::interactive::theme::{self, Depth, Style};

/// Rows the input surface takes: a blank row, the query, a blank row.
///
/// The blank rows are the surface, not spacing around it — they carry the same colour, which is
/// what makes the input read as a panel rather than as one coloured line. Three is codex's shape
/// and the smallest number that reads as deliberate: one row looks like a highlight, three looks
/// like somewhere to type.
const SURFACE_ROWS: usize = 3;

/// Unpainted rows below the surface, so the panel does not sit flush against the bottom edge.
const BOTTOM_MARGIN: usize = 1;

/// Unpainted columns either side of the surface, for the same reason.
const SIDE_MARGIN: usize = 1;

/// Everything the list does not get.
const CHROME_ROWS: usize = SURFACE_ROWS + BOTTOM_MARGIN;

/// What the frame needs to know about the world.
pub struct Frame<'a> {
    pub matches: &'a [Ranked],
    pub selected: usize,
    /// The first visible row, so a long list can scroll under a fixed window.
    pub offset: usize,
    pub query: &'a str,
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
        self.rows.saturating_sub(CHROME_ROWS).max(1)
    }
}

/// The whole screen, as one string of escapes.
pub fn frame(f: &Frame<'_>) -> String {
    let theme = theme::current();
    let depth = theme::depth();
    let pager = &theme.pager;
    let visible = f.visible_rows();

    let mut out = String::new();
    // Home, then draw downward. Every row erases to the end of the line as it goes, so a shorter
    // row cannot leave the tail of a longer one behind it.
    out.push_str("\x1b[H");

    // The list occupies the rows above the bar, and grows *upward*: the best match sits against
    // the separator. So the window is drawn bottom-up and any unused rows are at the top.
    let shown: Vec<&Ranked> = f.matches.iter().skip(f.offset).take(visible).collect();
    let blank_rows = visible.saturating_sub(shown.len());
    for _ in 0..blank_rows {
        out.push_str("\x1b[2K\r\n");
    }
    // **Drawn in reverse.** The list grows upward from the search bar, so the best match — index
    // 0 — is the row *nearest* the bar, which means it is painted last. Drawing them in order put
    // the selected row at the far end of the block from the cursor, which is the opposite of the
    // thing the layout exists to do.
    for (index, row) in shown.iter().enumerate().rev() {
        let absolute = f.offset + index;
        out.push_str("\x1b[2K");
        out.push_str(&list_row(
            row,
            absolute == f.selected,
            absolute % 2 == 1,
            f,
            pager,
            depth,
        ));
        out.push_str("\r\n");
    }

    // The input surface: three rows of one colour, the middle one carrying the query, inset from
    // both edges so the panel floats rather than sitting flush against them.
    let surface = pager.bg;
    let inner = f.cols.saturating_sub(SIDE_MARGIN * 2);
    let margin = " ".repeat(SIDE_MARGIN);
    let blank = Style {
        bg: surface,
        ..Style::default()
    };

    for row in 0..SURFACE_ROWS {
        out.push_str("\x1b[2K");
        out.push_str(&margin);
        if row == 1 {
            out.push_str(&search_bar(f, pager, surface, inner, depth));
        } else {
            out.push_str(&blank.paint(&" ".repeat(inner), depth));
        }
        out.push_str(&margin);
        out.push_str("\r\n");
    }

    // And a plain row under it, so the panel has air on all four sides.
    out.push_str("\x1b[2K");
    out
}

/// One command: the line, then how often, how long ago, and where.
///
/// The three annotations are right-aligned as a block so they form columns down the screen even
/// though the command text beside them varies wildly in length. That is the same reason the
/// completion dropdown aligns its info columns, and the same payoff: the eye can scan one column
/// without reading the others.
fn list_row(
    row: &Ranked,
    selected: bool,
    stripe: bool,
    f: &Frame<'_>,
    pager: &theme::Pager,
    depth: Depth,
) -> String {
    // **When and how often come first, then the command.** The two numbers are short and fixed
    // width, so leading with them gives the eye a ruler down the left of the screen; the command
    // is the variable-length thing and belongs after it. The directory is not shown at all — it
    // is still the third ranking signal, but it is the one you look at least and it was costing a
    // quarter of the width.
    let when_col = 5usize;
    let runs_col = 6usize;
    let marker_col = 2usize;
    let gaps = 2usize;
    let line_col = f
        .cols
        .saturating_sub(marker_col + when_col + runs_col + gaps + SIDE_MARGIN * 2)
        .max(8);

    let marker = if selected { "❯ " } else { "  " };
    let when = pad_left(&ago(f.now, row.command.last_at), when_col);
    let runs = pad_left(&format!("{}×", row.command.runs), runs_col);
    let line = pad_to_width(&truncate_to_width(&row.command.line, line_col), line_col);

    let text_style = if selected { pager.text_sel } else { pager.text };
    let meta_style = pager.column(1, selected);

    // Zebra striping: every other row takes the same colour the input surface uses, so a long
    // list can be read across without the eye losing its place. The selected row takes the
    // brighter selection colour, which is what distinguishes it from a merely-striped one.
    let row_bg = if selected {
        pager.sel_bg
    } else if stripe {
        pager.bg
    } else {
        None
    };
    let on_row = |style: Style| Style {
        bg: row_bg.or(style.bg),
        ..style
    };
    let pad = " ".repeat(SIDE_MARGIN);

    format!(
        "{}{}{} {} {}{}",
        on_row(Style::default()).paint(&pad, depth),
        on_row(pager.match_).paint(marker, depth),
        on_row(meta_style).paint(&when, depth),
        on_row(meta_style).paint(&runs, depth),
        on_row(text_style).paint(&line, depth),
        on_row(Style::default()).paint(&pad, depth),
    )
}

/// The query line, with the count of what matched on the right.
fn search_bar(
    f: &Frame<'_>,
    pager: &theme::Pager,
    surface: Option<theme::Color>,
    cols: usize,
    depth: Depth,
) -> String {
    let count = format!("{}/{}", f.matches.len(), f.total);
    let prompt = " ❯ ";
    let room = cols.saturating_sub(printed_width(prompt) + printed_width(&count) + 1);
    let typed = truncate_to_width(f.query, room);
    let gap = cols
        .saturating_sub(printed_width(prompt) + printed_width(&typed) + printed_width(&count) + 1);
    // Every part of the row takes the surface, the gap included: a panel with a hole in it is not
    // a panel.
    let on_surface = |style: Style| Style {
        bg: surface.or(style.bg),
        ..style
    };
    format!(
        "{}{}{}{}{}",
        on_surface(pager.match_).paint(prompt, depth),
        on_surface(pager.text_sel).paint(&typed, depth),
        on_surface(Style::default()).paint(&" ".repeat(gap), depth),
        on_surface(pager.column(1, false)).paint(&count, depth),
        on_surface(Style::default()).paint(" ", depth),
    )
}

/// Right-align `text` in exactly `width` cells, truncating if it does not fit.
///
/// Truncation matters as much as padding: a command run a million times renders `999999×`, which
/// is a cell wider than its column, and one cell of overflow wraps the row — after which every row
/// below it is one line out of place for the rest of the session.
fn pad_left(text: &str, width: usize) -> String {
    let text = truncate_to_width(text, width);
    let used = printed_width(&text);
    format!("{}{}", " ".repeat(width.saturating_sub(used)), text)
}

#[cfg(test)]
#[path = "render/tests.rs"]
mod tests;

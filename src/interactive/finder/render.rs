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
//!    ❯ car                                    12/840 [global]        ├ the surface, three rows
//!                                                                    ┘
//! ```
//!
//! **The list grows upward from the search bar.** The bar is at the bottom because that is where
//! the cursor is and where your eyes already are; the first result sits nearest it, across one
//! separating row, so the likely choice stays close without merging into the input. fzf and atuin
//! both settled on bottom-up results, and the reason is the same one.
//!
//! # The input and row rulers
//!
//! The input is a three-row surface: a blank row, the query, and a blank row, all sharing the
//! pager tint. It reaches both screen edges. The list is lighter-weight: even rows keep the
//! terminal background, odd rows use xterm colour 235 as a quiet ruler, and the selected row uses
//! the theme's selection colour. Every cell in a coloured row is painted, including the spaces
//! between columns, so the ruler remains continuous.
//!
//! The input colour is the dropdown's own — `oslo.theme.pager.bg` — so a theme still controls the
//! shell's two input surfaces together. The scope badge is deliberately stronger: foreground 0
//! on accent background 1.

use super::Scope;
use super::rank::{Ranked, ago};
use crate::interactive::dropdown::width::{pad_to_width, truncate_to_width};
use crate::interactive::prompt::printed_width;
use crate::interactive::theme::{self, Color, Depth, Style};

/// Cells the drawn cursor takes in the search bar.
const CURSOR_WIDTH: usize = 1;

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

/// The list keeps a little horizontal air. The input does not: its surface spans the full width.
const LIST_SIDE_MARGIN: usize = 1;

/// The alternating finder row. Kept separate from `pager.bg`: that colour belongs to the input
/// surface and the completion dropdown, while this is only a quiet ruler across history rows.
const STRIPE_BG: Color = Color::Indexed(235);

/// Everything the list does not get.
const CHROME_ROWS: usize = TOP_MARGIN + SURFACE_ROWS + BOTTOM_MARGIN;

/// What the frame needs to know about the world.
pub struct Frame<'a> {
    pub matches: &'a [Ranked],
    pub selected: usize,
    /// The first visible row, so a long list can scroll under a fixed window.
    pub offset: usize,
    pub query: &'a str,
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

    // A real gap above the surface. Its own upper blank row is tinted and therefore reads as part
    // of the input, not as separation from the result list.
    out.push_str("\x1b[2K\r\n");

    // The input surface: three full-width rows of one colour, the middle one carrying the query.
    let surface = pager.bg;
    let blank = Style {
        bg: surface,
        ..Style::default()
    };

    for row in 0..SURFACE_ROWS {
        out.push_str("\x1b[2K");
        if row == 1 {
            out.push_str(&search_bar(f, pager, surface, f.cols, depth));
        } else {
            out.push_str(&blank.paint(&" ".repeat(f.cols), depth));
        }
        out.push_str("\r\n");
    }

    // And a plain row under it, so the panel does not sit on the terminal edge.
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
        .saturating_sub(marker_col + when_col + runs_col + gaps + LIST_SIDE_MARGIN * 2)
        .max(8);

    let marker = if selected { "❯ " } else { "  " };
    let when = pad_left(&ago(f.now, row.command.last_at), when_col);
    let runs = pad_left(&format!("{}×", row.command.runs), runs_col);
    let shown = truncate_to_width(&row.command.line, line_col);
    let line = pad_to_width(&shown, line_col);

    let text_style = if selected { pager.text_sel } else { pager.text };
    let meta_style = pager.column(1, selected);

    // Zebra striping: every other row takes a quiet fixed grey, so a long list can be read across
    // without the eye losing its place. The selected row takes the brighter selection colour.
    let row_bg = if selected {
        pager.sel_bg
    } else if stripe {
        Some(STRIPE_BG)
    } else {
        None
    };
    let on_row = |style: Style| Style {
        bg: row_bg.or(style.bg),
        ..style
    };
    let pad = " ".repeat(LIST_SIDE_MARGIN);
    // These gaps must be painted too. Literal spaces between independently styled columns punch
    // terminal-background holes through selected and striped rows.
    let gap = on_row(Style::default()).paint(" ", depth);

    format!(
        "{}{}{}{}{}{}{}{}",
        on_row(Style::default()).paint(&pad, depth),
        on_row(pager.match_).paint(marker, depth),
        on_row(meta_style).paint(&when, depth),
        gap,
        on_row(meta_style).paint(&runs, depth),
        on_row(Style::default()).paint(" ", depth),
        highlight_matches(&line, &shown, f.query, on_row(text_style), depth),
        on_row(Style::default()).paint(&pad, depth),
    )
}

/// Paint `line`, marking the characters the query matched.
///
/// **Only the characters that matched**, not the row and not the word around them. A fuzzy hit is
/// otherwise a mystery: five rows come back and nothing says which letters put them there.
///
/// The mark is the accent on colour 1 with colour 0 over it — the terminal's own palette, so it
/// belongs to whatever scheme is in use, and inverted enough to read at a glance against both the
/// striped and the selected background.
fn highlight_matches(padded: &str, shown: &str, query: &str, base: Style, depth: Depth) -> String {
    let marks = crate::interactive::matching::positions(shown, query.trim());
    if marks.is_empty() {
        return base.paint(padded, depth);
    }
    // The mark keeps the row's background nowhere: it *is* a background, which is what makes it
    // visible on a selected row as well as a plain one.
    //
    // **Not bold.** Bold is a colour hint as much as a weight: many terminals render it by
    // switching to the bright palette, and some to plain grey — which would take the foreground
    // off colour 0 and undo the contrast this pair exists for. The inversion is the emphasis.
    let marked = Style {
        fg: Some(Color::Indexed(0)),
        bg: Some(Color::Indexed(1)),
        ..Style::default()
    };
    let mut out = String::new();
    let mut run = String::new();
    for (at, ch) in padded.char_indices() {
        let hit = marks.contains(&at);
        // Runs are painted together, so a contiguous match is one escape rather than one per
        // character — which matters on a screen of rows redrawn per keystroke.
        if hit {
            if !run.is_empty() {
                out.push_str(&base.paint(&run, depth));
                run.clear();
            }
            out.push_str(&marked.paint(&ch.to_string(), depth));
        } else {
            run.push(ch);
        }
    }
    if !run.is_empty() {
        out.push_str(&base.paint(&run, depth));
    }
    out
}

/// The query line, with the count and current search scope on the right.
fn search_bar(
    f: &Frame<'_>,
    pager: &theme::Pager,
    surface: Option<theme::Color>,
    cols: usize,
    depth: Depth,
) -> String {
    let scope = match f.scope {
        Scope::Global => "[global]",
        Scope::Local => "[local]",
    };
    let count = format!("{}/{}", f.matches.len(), f.total);
    let prompt = " ❯ ";
    let room = cols
        .saturating_sub(printed_width(prompt) + printed_width(&count) + printed_width(scope) + 2);
    // One cell is kept back for the cursor, which is part of the input and has to fit.
    let typed = truncate_to_width(f.query, room.saturating_sub(1));
    let gap = cols.saturating_sub(
        printed_width(prompt)
            + printed_width(&typed)
            + CURSOR_WIDTH
            + printed_width(&count)
            + printed_width(scope)
            + 2,
    );
    // Every part of the row takes the surface, the gap included: a panel with a hole in it is not
    // a panel.
    let on_surface = |style: Style| Style {
        bg: surface.or(style.bg),
        ..style
    };
    format!(
        "{}{}{}{}{}{}{}{}",
        on_surface(pager.match_).paint(prompt, depth),
        on_surface(pager.text_sel).paint(&typed, depth),
        // **A cursor.** The real one is hidden — the finder owns the alternate screen — so the
        // caret is drawn into the frame as a reversed block, the same way every widget in
        // `interactive::ask` does it. Without it the search box gave no sign it was taking keys.
        Style {
            fg: Some(Color::Indexed(0)),
            bg: Some(Color::Indexed(1)),
            ..Style::default()
        }
        .paint(" ", depth),
        on_surface(Style::default()).paint(&" ".repeat(gap), depth),
        on_surface(pager.column(1, false)).paint(&count, depth),
        on_surface(Style::default()).paint(" ", depth),
        Style {
            fg: Some(Color::Indexed(0)),
            bg: Some(Color::Indexed(1)),
            ..Style::default()
        }
        .paint(scope, depth),
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

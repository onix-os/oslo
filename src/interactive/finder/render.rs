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
//!  ┌───────────────────────────────────────────────── the list, newest or best-matching at the
//!  │ git status                          3× │ 2h │ ~/src/oslo        bottom, growing upward
//!  │ cargo build --release              41× │ 5h │ ~/src/oslo
//!  │▌cargo test                        118× │ 1d │ ~/src/oslo        ← selected
//!  ├─────────────────────────────────────────────────
//!  │ ❯ car                                        12/840             the query, and the count
//!  └─────────────────────────────────────────────────
//! ```
//!
//! **The list grows upward from the search bar.** The bar is at the bottom because that is where
//! the cursor is and where your eyes already are; the first result sits directly above it, so the
//! thing you are most likely to take is the thing nearest what you are typing. fzf and atuin both
//! settled here, and the reason is the same one.

use super::rank::{Ranked, ago};
use crate::interactive::dropdown::width::{pad_to_width, truncate_to_width};
use crate::interactive::prompt::printed_width;
use crate::interactive::theme::{self, Depth, Style};

/// Rows the chrome takes: the separator and the search bar.
const CHROME_ROWS: usize = 2;

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
    /// Where the shell is, so the directory column can be shortened against `$HOME` and the local
    /// rows marked.
    pub home: &'a str,
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
        out.push_str(&list_row(row, absolute == f.selected, f, pager, depth));
        out.push_str("\r\n");
    }

    out.push_str("\x1b[2K");
    out.push_str(&separator(f.cols, pager.scroll, depth));
    out.push_str("\r\n");

    out.push_str("\x1b[2K");
    out.push_str(&search_bar(f, pager, depth));
    out
}

/// `─────` across the width, in the dim colour the dropdown uses for its scroll marker.
fn separator(cols: usize, style: Style, depth: Depth) -> String {
    style.paint(&"─".repeat(cols), depth)
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
    f: &Frame<'_>,
    pager: &theme::Pager,
    depth: Depth,
) -> String {
    let runs = format!("{}×", row.command.runs);
    let when = ago(f.now, row.command.last_at);
    let dir = shorten(&row.command.dir, f.home);

    // Widths for the fixed columns, with a gap between each.
    let runs_col = 7usize;
    let when_col = 5usize;
    let dir_col = (f.cols / 4).clamp(10, 40);
    // What is left over is the command's, minus the marker and the gaps.
    let marker_col = 2usize;
    let gaps = 3usize;
    let line_col = f
        .cols
        .saturating_sub(marker_col + runs_col + when_col + dir_col + gaps)
        .max(8);

    let marker = if selected { "▌ " } else { "  " };
    let line = pad_to_width(&truncate_to_width(&row.command.line, line_col), line_col);
    let runs = pad_left(&runs, runs_col);
    let when = pad_left(&when, when_col);
    let dir = pad_to_width(&truncate_to_width(&dir, dir_col), dir_col);

    let text_style = if selected { pager.text_sel } else { pager.text };
    let meta_style = pager.column(1, selected);
    // A directory you are standing in is worth marking: it is the third ranking signal, so seeing
    // *why* a row is high in the list should not require guessing.
    let dir_style = if row.here {
        pager.column(0, selected)
    } else {
        meta_style
    };

    // The whole row carries one background, so the columns read as a block rather than as four
    // separately-coloured strips. Same reason the completion dropdown does it.
    let row_bg = if selected { pager.sel_bg } else { pager.bg };
    let on_row = |style: Style| Style {
        bg: row_bg.or(style.bg),
        ..style
    };

    format!(
        "{}{} {} {} {}",
        on_row(text_style).paint(marker, depth),
        on_row(text_style).paint(&line, depth),
        on_row(meta_style).paint(&runs, depth),
        on_row(meta_style).paint(&when, depth),
        on_row(dir_style).paint(&dir, depth),
    )
}

/// The query line, with the count of what matched on the right.
fn search_bar(f: &Frame<'_>, pager: &theme::Pager, depth: Depth) -> String {
    let count = format!("{}/{}", f.matches.len(), f.total);
    let prompt = " ❯ ";
    let room = f
        .cols
        .saturating_sub(printed_width(prompt) + printed_width(&count) + 1);
    let typed = truncate_to_width(f.query, room);
    let gap = f
        .cols
        .saturating_sub(printed_width(prompt) + printed_width(&typed) + printed_width(&count) + 1);
    format!(
        "{}{}{}{}",
        pager.match_.paint(prompt, depth),
        pager.text_sel.paint(&typed, depth),
        " ".repeat(gap),
        pager.column(1, false).paint(&count, depth),
    )
}

/// Right-align `text` in `width` cells.
fn pad_left(text: &str, width: usize) -> String {
    let used = printed_width(text);
    format!("{}{}", " ".repeat(width.saturating_sub(used)), text)
}

/// `$HOME` written as `~`, because the directory column is narrow and the prefix is the least
/// informative part of every path in it.
fn shorten(path: &str, home: &str) -> String {
    if home.is_empty() || !path.starts_with(home) {
        return path.to_string();
    }
    match path.len() == home.len() {
        true => "~".to_string(),
        false if path.as_bytes().get(home.len()) == Some(&b'/') => {
            format!("~{}", &path[home.len()..])
        }
        false => path.to_string(),
    }
}

#[cfg(test)]
#[path = "render/tests.rs"]
mod tests;

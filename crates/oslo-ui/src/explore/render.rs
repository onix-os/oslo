//! Drawing one frame of the explorer.
//!
//! Pure: a `Frame` in, a string out, no terminal touched. So the layout can be tested — which
//! matters more here than in a widget that draws a list, because two viewports have to agree
//! (which rows are visible, and which columns) and getting either wrong is invisible in a
//! screenshot and obvious the moment you scroll.
//!
//! # The layout
//!
//! ```text
//!                                                              ← screen margin
//!  name            meta        tags   explore › meta  3/16     ← header band, on the surface
//!  > tmpfs         <2 fields>  <3 items>                       ← the cursor is one cell
//!    /dev/nvme0n1  <2 fields>  <1 item>                        ← every other row striped
//!                                                              ← screen margin
//! ```
//!
//! The stripe, the marker, the full-width rows and every colour come from [`crate::ask::look`] and
//! specifically from [`Preset::History`], so the viewer and the history finder cannot drift apart
//! about what a list looks like.
//!
//! **Two of that preset's parts are deliberately unused.** `reverse` grows a list upward so the
//! best match sits against the cursor, which is right when rank is the answer and wrong for a table
//! whose row order is data. And the tinted filter surface is not drawn at all: filtering rows is
//! what `where` is for, and a viewer with its own search would be a second way to do it that only
//! works while you are looking.

use super::{Cell, Sheet};
use crate::ask::look::{Look, Preset, Width};
use crate::dropdown::width::{display_width, pad_to_width, truncate_to_width};
use crate::theme::{self, Style};

/// The widest a single column is drawn.
///
/// A `cmdline` is a hundred characters and would own the screen. Wider than the drawn table's 60
/// because there is a whole terminal here rather than one line of a transcript, and because the
/// cell you are reading can always be opened.
const WIDEST: usize = 40;

/// Blank columns between two columns of the table.
const GAP: usize = 2;

/// An unpainted row at each end, so the block does not sit flush against either edge.
const SCREEN_MARGIN: usize = 1;

/// The band the column names sit on.
const HEADER_ROWS: usize = 1;

/// The finder's look, with the list the right way up.
pub(super) fn look() -> Look {
    Look {
        reverse: false,
        ..Preset::History.look()
    }
}

/// Everything one frame is drawn from.
pub(super) struct Frame<'a> {
    pub sheet: &'a Sheet,
    pub row: usize,
    pub column: usize,
    pub top: usize,
    pub left: usize,
    /// The titles of the levels above this one, outermost first.
    pub trail: &'a [String],
    pub cols: usize,
    pub rows: usize,
}

/// One width per column, measured over every row — not over the visible ones.
///
/// Measuring the window would make the columns twitch as you scroll, which is the one thing a table
/// must not do: the whole reason to draw a table rather than a list is that a value stays under its
/// header while your eye moves down.
pub(super) fn widths(sheet: &Sheet) -> Vec<usize> {
    sheet
        .columns
        .iter()
        .enumerate()
        .map(|(i, name)| {
            sheet
                .rows
                .iter()
                .filter_map(|row| row.get(i))
                .map(|cell| display_width(cell.text()))
                .chain(std::iter::once(display_width(name)))
                .max()
                .unwrap_or(0)
                .min(WIDEST)
        })
        .collect()
}

/// How many columns starting at `left` fit in `room` cells.
///
/// **At least one, always.** A column wider than the screen would otherwise fit zero of them and
/// draw an empty frame, which looks exactly like a table with no data in it.
pub(super) fn fitting(widths: &[usize], left: usize, room: usize) -> usize {
    let mut used = 0usize;
    let mut count = 0usize;
    for width in widths.iter().skip(left) {
        let would = used + width + if count > 0 { GAP } else { 0 };
        if count > 0 && would > room {
            break;
        }
        used = would;
        count += 1;
    }
    count.max(1)
}

/// How many data rows there is room for, given the chrome around them.
pub(super) fn window(rows: usize) -> usize {
    rows.saturating_sub(SCREEN_MARGIN * 2 + HEADER_ROWS).max(1)
}

/// Cells across that a row of the table may use.
///
/// The look's margin and padding and the selection marker all come off the front, and the same
/// arithmetic decides where the header's columns start — so a column name sits over its values.
fn room(look: &Look, cols: usize) -> usize {
    let taken = look.margin + look.pad * 2 + display_width(&look.marker);
    cols.saturating_sub(taken).max(1)
}

/// The whole screen, cursor at the top left and every line cleared as it is written.
pub(super) fn frame(f: &Frame) -> String {
    let depth = theme::depth();
    let look = look();
    let measured = widths(f.sheet);
    let visible = fitting(&measured, f.left, room(&look, f.cols));
    let height = window(f.rows);
    let margin = " ".repeat(look.margin);
    let across = f.cols.saturating_sub(look.margin);

    let mut lines: Vec<String> = vec![String::new()];
    lines.push(header(f, &look, &measured, visible, depth));
    for at in 0..height {
        lines.push(match f.sheet.rows.get(f.top + at) {
            Some(_) => body(f, &look, &measured, f.top + at, visible, depth),
            // A row with nothing on it still wears what the list is painted on, so a half-empty
            // table is a block with space in it rather than one that stops early.
            None => look.row.paint(&" ".repeat(across), depth),
        });
    }
    lines.push(String::new());

    // **Nothing is cut here.** Every line above was measured and cut *before* it was painted;
    // `truncate_to_width` counts escapes correctly but cuts by grapheme, so a second pass over a
    // painted line could sever an escape and leave the colour running to the bottom of the screen.
    let mut out = String::from("\x1b[H");
    for (at, line) in lines.iter().enumerate() {
        out.push_str("\r\x1b[K");
        out.push_str(&margin);
        out.push_str(line);
        // No newline after the last row: one more would scroll the alternate screen by a line and
        // push the header off the top.
        if at + 1 < lines.len() {
            out.push_str("\r\n");
        }
    }
    out
}

/// The column names, over the columns they name, with where you are at the right end.
///
/// One band rather than a header and a status line: it is the only furniture the viewer has, and
/// two rows of chrome to say what one can is two rows the table does not get.
fn header(
    f: &Frame,
    look: &Look,
    measured: &[usize],
    visible: usize,
    depth: theme::Depth,
) -> String {
    let on = |style: Style| Style {
        bg: look.surface.or(style.bg),
        ..style
    };
    let mut line = " ".repeat(look.pad + display_width(&look.marker));
    for (at, width) in measured.iter().enumerate().skip(f.left).take(visible) {
        if at > f.left {
            line.push_str(&" ".repeat(GAP));
        }
        line.push_str(&place(f.sheet, at, &f.sheet.columns[at], *width));
    }

    let mut trail: Vec<&str> = f.trail.iter().map(String::as_str).collect();
    trail.push(&f.sheet.title);
    // The column span is only worth saying when some of them are off the screen: on a table that
    // fits, `‹ 1-6/6 ›` is noise about a fact the screen is already showing.
    let span = match visible >= measured.len() {
        true => String::new(),
        false => format!(
            "  ‹ {}-{}/{} ›",
            f.left + 1,
            f.left + visible,
            measured.len()
        ),
    };
    let where_you_are = format!(
        "{}  {}/{}{span} ",
        trail.join(" › "),
        f.row + 1,
        f.sheet.rows.len()
    );

    // **The padding goes first, then the text.** A column name is padded out to its column's width,
    // so the line ends in whatever trailing space the last column has — and cutting the line to
    // make room for the position put an ellipsis in the middle of that blank, which reads as a name
    // that was truncated when nothing was. Trimmed first, the marker only appears when a name
    // really did not fit.
    let across = f.cols.saturating_sub(look.margin);
    let room = across.saturating_sub(display_width(&where_you_are));
    let names = truncate_to_width(line.trim_end(), room);
    format!(
        "{}{}",
        on(look.meta_style).paint(&pad_to_width(&names, room), depth),
        on(look.muted).paint(&where_you_are, depth)
    )
}

/// One row of the table: the marker, then a cell per visible column.
fn body(
    f: &Frame,
    look: &Look,
    measured: &[usize],
    index: usize,
    visible: usize,
    depth: theme::Depth,
) -> String {
    let current = index == f.row;
    // **The cursor is a cell, so the row is not highlighted.** A list has one dimension and the
    // finder can paint the whole of it; here the thing you are pointing at is one column of one
    // row, and painting the row would say the row was picked. The marker says which row it is.
    //
    // The stripe belongs to the row's place in the list, so the absolute index decides it —
    // measured from the visible window the stripes would crawl as the table scrolled.
    let base = match look.stripe.filter(|_| index % 2 == 1) {
        Some(stripe) => Style {
            bg: stripe.bg.or(look.row.bg),
            ..look.row
        },
        None => look.row,
    };
    let on = |style: Style| Style {
        bg: base.bg.or(style.bg),
        ..style
    };

    let pad = " ".repeat(look.pad);
    let marker = match current {
        true => on(look.accent).paint(&look.marker, depth),
        false => on(base).paint(&" ".repeat(display_width(&look.marker)), depth),
    };

    let row = &f.sheet.rows[index];
    let mut cells = String::new();
    let mut used = 0usize;
    for (at, width) in measured.iter().enumerate().skip(f.left).take(visible) {
        if at > f.left {
            cells.push_str(&on(base).paint(&" ".repeat(GAP), depth));
            used += GAP;
        }
        let cell = row.get(at);
        let text = place(f.sheet, at, cell.map(Cell::text).unwrap_or(""), *width);
        used += display_width(&text);
        // Two marks, and they say different things. **Accent** is a cell with a table under it,
        // wherever it is on the screen; the **selection background** is the one cell Enter would
        // open. A cell that is both is both, which is the common case.
        let style = match cell.and_then(Cell::sheet).is_some() {
            true => on(look.accent),
            false => on(base),
        };
        let style = match current && at == f.column {
            true => Style {
                bg: look.selected.bg.or(style.bg),
                ..style
            },
            false => style,
        };
        cells.push_str(&style.paint(&text, depth));
    }
    // Padded out on the row's own colour, which is what makes the stripe read as a ruler across
    // the screen rather than as a coloured word — the argument `Width::Full` makes in the look.
    let rest = match look.width {
        Width::Full => room(look, f.cols).saturating_sub(used),
        Width::Content => 0,
    };
    format!(
        "{}{}{}{}{}",
        on(base).paint(&pad, depth),
        marker,
        cells,
        on(base).paint(&" ".repeat(rest), depth),
        on(base).paint(&pad, depth),
    )
}

/// One cell's text, cut to its column and padded to the side the column reads from.
fn place(sheet: &Sheet, at: usize, text: &str, width: usize) -> String {
    let cut = truncate_to_width(text, width);
    match sheet.numeric.get(at) {
        Some(true) => format!(
            "{}{cut}",
            " ".repeat(width.saturating_sub(display_width(&cut)))
        ),
        _ => pad_to_width(&cut, width),
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

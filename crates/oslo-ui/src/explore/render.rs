//! Drawing one frame of the explorer.
//!
//! Pure: a `Frame` in, a string out, no terminal touched. So the layout can be tested — which
//! matters more here than in a widget that draws a list, because two viewports have to agree
//! (which rows are visible, and which columns) and getting either wrong is invisible in a
//! screenshot and obvious the moment you scroll.

use super::{Cell, Sheet};
use crate::dropdown::width::{display_width, pad_to_width, truncate_to_width};
use crate::theme::{self, Style};

/// The widest a single column is drawn.
///
/// A `cmdline` is a hundred characters and would own the screen. Wider than the drawn table's 60
/// because there is a whole terminal here rather than one line of a transcript, and because the
/// cell you are reading can always be opened.
const WIDEST: usize = 40;

/// Everything one frame is drawn from.
pub(super) struct Frame<'a> {
    pub sheet: &'a Sheet,
    /// Indices into `sheet.rows`, after filtering. The cursor and the viewport are positions in
    /// *this*, not in the sheet, so narrowing the list cannot leave the cursor on a hidden row.
    pub shown: &'a [usize],
    pub row: usize,
    pub column: usize,
    pub top: usize,
    pub left: usize,
    pub query: &'a str,
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

/// Blank columns between two columns of the table.
const GAP: usize = 2;

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

/// How many data rows there is room for, given the chrome this frame draws.
pub(super) fn window(rows: usize, query: &str) -> usize {
    let chrome = 1 + 1 + 1 + 1 + usize::from(!query.is_empty());
    rows.saturating_sub(chrome).max(1)
}

/// The whole screen, cursor positioned at the top left and every line cleared as it is written.
pub(super) fn frame(f: &Frame) -> String {
    let ui = theme::current().ui;
    let depth = theme::depth();
    let widths = widths(f.sheet);
    let visible = fitting(&widths, f.left, f.cols);
    let height = window(f.rows, f.query);

    let mut out = String::from("\x1b[H");
    let line = |text: &str, out: &mut String| {
        out.push_str("\x1b[2K");
        out.push_str(&truncate_to_width(text, f.cols));
        out.push_str("\r\n");
    };

    line(
        &heading(f, &ui, depth, f.left, visible, widths.len()),
        &mut out,
    );
    line(&header(f, &widths, depth, ui.question), &mut out);
    line(&ui.muted.paint(&"─".repeat(f.cols), depth), &mut out);
    for at in 0..height {
        let text = match f.shown.get(f.top + at) {
            Some(index) => body(f, &widths, depth, *index, f.top + at == f.row, visible),
            None => String::new(),
        };
        line(&text, &mut out);
    }
    if !f.query.is_empty() {
        line(
            &format!(
                "{} {}",
                ui.accent.paint("/", depth),
                ui.question.paint(f.query, depth)
            ),
            &mut out,
        );
    }
    // No trailing newline on the last row: one more would scroll the alternate screen by a line and
    // push the heading off the top.
    out.push_str("\x1b[2K");
    out.push_str(&truncate_to_width(&legend(f), f.cols));
    out
}

/// The breadcrumb, and where the cursor is in a table you can only see part of.
fn heading(
    f: &Frame,
    ui: &theme::Ui,
    depth: theme::Depth,
    left: usize,
    visible: usize,
    of: usize,
) -> String {
    let mut trail: Vec<&str> = f.trail.iter().map(String::as_str).collect();
    trail.push(&f.sheet.title);
    let rows = match f.shown.len() {
        0 => "no rows".to_string(),
        n => format!("row {}/{n}", f.row + 1),
    };
    // The column span is only worth saying when some of them are off the screen; on a table that
    // fits, "col 1-6/6" is noise about a fact the screen is already showing.
    let columns = match visible >= of {
        true => String::new(),
        false => format!("  col {}-{}/{of}", left + 1, left + visible),
    };
    format!(
        "{}  {}",
        ui.question.paint(&trail.join(" › "), depth),
        ui.muted.paint(&format!("{rows}{columns}"), depth)
    )
}

/// The column names, over the columns they name.
fn header(f: &Frame, widths: &[usize], depth: theme::Depth, style: Style) -> String {
    let ui = theme::current().ui;
    let visible = fitting(widths, f.left, f.cols);
    let mut line = String::new();
    for (at, width) in widths.iter().enumerate().skip(f.left).take(visible) {
        if at > f.left {
            line.push_str(&" ".repeat(GAP));
        }
        let name = place(f.sheet, at, &f.sheet.columns[at], *width);
        // The column the cursor is in is named in the accent colour, so which cell Enter would open
        // is readable from the header as well as from the row.
        line.push_str(&match at == f.column {
            true => ui.accent.paint(&name, depth),
            false => style.paint(&name, depth),
        });
    }
    line
}

/// One row of the table, with the cursor's own cell picked out of it.
fn body(
    f: &Frame,
    widths: &[usize],
    depth: theme::Depth,
    index: usize,
    current: bool,
    visible: usize,
) -> String {
    let ui = theme::current().ui;
    let row = &f.sheet.rows[index];
    let mut line = String::new();
    for (at, width) in widths.iter().enumerate().skip(f.left).take(visible) {
        if at > f.left {
            line.push_str(&" ".repeat(GAP));
        }
        let cell = row.get(at);
        let text = place(f.sheet, at, cell.map(Cell::text).unwrap_or(""), *width);
        // Three states, and they have to be distinguishable from each other rather than merely
        // from plain text: the cell under the cursor is reversed, the rest of its row is accented,
        // and a cell you could open is underlined wherever it is.
        let style = match (current && at == f.column, current) {
            (true, _) => Style {
                reverse: true,
                ..ui.accent
            },
            (_, true) => ui.accent,
            _ => Style::default(),
        };
        let style = match cell.and_then(Cell::sheet).is_some() {
            true => Style {
                underline: true,
                ..style
            },
            false => style,
        };
        line.push_str(&style.paint(&text, depth));
    }
    line
}

/// What the keys do, and only the ones that do something here.
fn legend(f: &Frame) -> String {
    let mut keys = vec![("↑↓←→", "move")];
    if f.shown
        .get(f.row)
        .and_then(|index| f.sheet.rows[*index].get(f.column))
        .and_then(Cell::sheet)
        .is_some()
    {
        keys.push(("enter", "open"));
    }
    if !f.trail.is_empty() {
        keys.push(("bksp", "back"));
    }
    keys.push(("type", "filter"));
    keys.push(("esc", "quit"));
    crate::ask::chrome::legend_text(&keys)
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

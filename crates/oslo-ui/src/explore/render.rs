//! Drawing one frame of the explorer.
//!
//! Pure: a `Frame` in, a string out, no terminal touched. So the layout can be tested — which
//! matters more here than in a widget that draws a list, because two viewports have to agree
//! (which rows are visible, and which columns) and getting either wrong is invisible in a
//! screenshot and obvious the moment you scroll.
//!
//! # The look is the history finder's
//!
//! ```text
//!                                                              ← screen margin
//!  name            meta        tags                  ‹ 1-3/5 › ← header band, on the surface
//!  > tmpfs         <2 fields>  <3 items>                       ← the cursor is one cell
//!    /dev/nvme0n1  <2 fields>  <1 item>                        ← every other row striped
//!                                                              ← gap
//!  ⬝⬝⬝⬝⬝⬝⬝⬝⬝  >>  type to filter    [explore › meta] || 3/16   ├ the surface
//!                                                              ┘
//!  ↑↓←→ move • enter open • bksp back • esc quit               ← legend
//! ```
//!
//! Every part of that except the header band and the legend comes from [`crate::ask::look`], and
//! specifically from [`Preset::History`] — the striping, the marker, the full-width rows, the
//! three-row tinted surface, the `>>` prompt, the sweep, the badge and the counter. A viewer that
//! painted its own search bar would be a second thing to keep in step with the finder's, and the
//! two would drift the way the finder and `ui filter` did before the look existed.
//!
//! **`reverse` is the one thing turned off.** The finder grows its list upward so the best match
//! sits against the cursor, because rank is what a search answers. A table's row order is *data* —
//! `sort-by` put it there, or the producer did — so it reads top-down.

use super::{Cell, Sheet};
use crate::ask::look::{Look, Preset, View, Where, Width};
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

/// Unpainted row at the top, matching the one the surface leaves at the bottom.
const SCREEN_MARGIN: usize = 1;

/// The header band, and the key legend under the surface.
const HEADER_ROWS: usize = 1;
const LEGEND_ROWS: usize = 1;

/// The finder's look, with the list the right way up.
pub(super) fn look() -> Look {
    Look {
        reverse: false,
        filter_at: Where::Bottom,
        // `{index}/{n}` rather than the finder's `{n}/{total}`: a table is not a search result, so
        // where you are in it is the useful number and how many the filter left is the context.
        right: "{badge} || {index}/{n} ".to_string(),
        ..Preset::History.look()
    }
}

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
    /// How long the viewer has been open, for the sweep in the search bar. Passed in rather than
    /// read from a clock, so a frame stays a pure function of its input.
    pub elapsed_ms: u64,
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
    let chrome = SCREEN_MARGIN + HEADER_ROWS + look().extra_rows(true) + LEGEND_ROWS;
    rows.saturating_sub(chrome).max(1)
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
    // The badge is the one part of the bar that is a *fact about where you are* rather than about
    // what you are looking at, which is what the finder puts its scope in. Here that is the trail.
    let look = Look {
        badge: trail(f),
        // `1/0` is what `{index}/{n}` says about a filter that matched nothing, and it reads as a
        // position in a list rather than as the absence of one.
        right: match f.shown.is_empty() {
            true => "{badge} || no match ".to_string(),
            false => look().right,
        },
        ..look()
    };
    let measured = widths(f.sheet);
    let visible = fitting(&measured, f.left, room(&look, f.cols));
    let height = window(f.rows);
    let margin = " ".repeat(look.margin);

    let across = f.cols.saturating_sub(look.margin);
    let mut lines: Vec<String> = vec![String::new()];
    lines.push(header(f, &look, &measured, visible, depth));
    for at in 0..height {
        lines.push(match f.shown.get(f.top + at) {
            Some(index) => body(
                f,
                &look,
                &measured,
                *index,
                f.top + at == f.row,
                visible,
                depth,
            ),
            // A row with nothing on it still wears what the list is painted on, so a half-empty
            // table is a block with space in it rather than one that stops early.
            None => look.row.paint(&" ".repeat(across), depth),
        });
    }
    // The gap and the three-row surface, drawn by the same code as the finder's.
    lines.extend(look.rows(&[], &view(f, &look)));
    // Indented by the same pad the rows and the search bar carry, so the three left edges agree.
    let keys = format!("{}{}", " ".repeat(look.pad), legend(f));
    lines.push(look.muted.paint(&truncate_to_width(&keys, across), depth));

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

/// What the shared list renderer needs to know to draw the bar.
///
/// `height: 0` is what asks it for the bar alone: the list above is a table this module draws
/// itself, because the cursor here is a *cell* and `Row` has no notion of one.
fn view<'a>(f: &'a Frame, look: &Look) -> View<'a> {
    View {
        selected: f.row,
        offset: f.top,
        height: 0,
        query: f.query,
        matched: f.shown.len(),
        total: f.sheet.rows.len(),
        marked: 0,
        cols: f.cols.saturating_sub(look.margin),
        filtering: true,
        elapsed_ms: f.elapsed_ms,
    }
}

/// The column names, over the columns they name, on the same tint as the search bar.
///
/// A band rather than a rule: the surface at the other end of the screen is what says "this is the
/// widget's own furniture, not your data", and using it twice frames the rows between them.
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
    // Where you are in a table you can only see part of. Said only when some of it is off the
    // screen: on a table that fits, `‹ 1-6/6 ›` is noise about a fact the screen already shows.
    let span = match visible >= measured.len() {
        true => String::new(),
        false => format!("‹ {}-{}/{} ›", f.left + 1, f.left + visible, measured.len()),
    };
    // **The padding goes first, then the text.** A column name is padded out to its column's width,
    // so the line ends in whatever trailing space the last column has — and cutting the line to
    // make room for the span put an ellipsis in the middle of that blank, which reads as a name
    // that was truncated when nothing was. Trimmed first, the marker only appears when a name
    // really did not fit.
    let across = f.cols.saturating_sub(look.margin);
    let room = across.saturating_sub(display_width(&span));
    let names = truncate_to_width(line.trim_end(), room);
    format!(
        "{}{}",
        on(look.meta_style).paint(&pad_to_width(&names, room), depth),
        on(look.muted).paint(&span, depth)
    )
}

/// One row of the table: the marker, then a cell per visible column.
fn body(
    f: &Frame,
    look: &Look,
    measured: &[usize],
    index: usize,
    current: bool,
    visible: usize,
    depth: theme::Depth,
) -> String {
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
        // wherever it is on the screen; the **selection**, background and underline both, is the
        // one cell Enter would open. A cell that is both is both, which is the common case.
        let style = match cell.and_then(Cell::sheet).is_some() {
            true => on(look.accent),
            false => on(base),
        };
        let style = match current && at == f.column {
            true => Style {
                underline: true,
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

/// The breadcrumb, for the badge on the search bar.
pub(super) fn trail(f: &Frame) -> String {
    let mut trail: Vec<&str> = f.trail.iter().map(String::as_str).collect();
    trail.push(&f.sheet.title);
    format!(" {} ", trail.join(" › "))
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
    keys.push(("esc", "quit"));
    crate::ask::chrome::legend_text(&keys)
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

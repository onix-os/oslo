//! Turning a list and a [`Look`] into the rows of a frame.
//!
//! One renderer for every widget that shows a list, which is what stops `choose`, `table` and
//! `file` from drifting into three subtly different lists. Each of them says what its rows *say*;
//! this decides what they look like.
//!
//! # Painting the gaps
//!
//! Every cell of a coloured row is painted, the spaces between columns included. Literal spaces
//! between independently styled runs punch terminal-background holes through a selected or striped
//! row, and a row with holes in it does not read as a row.

use super::{Look, Where, Width};
use crate::ui::dropdown::width::{pad_to_width, truncate_to_width};
use crate::ui::prompt::printed_width;
use crate::ui::theme::{self, Style};

/// One line of a list, as the widget knows it.
pub struct Row {
    /// The label. Painted, highlighted and truncated here.
    pub text: String,
    /// A column before the text that belongs to the row rather than to the cursor — a checkbox, a
    /// file's kind. Empty for a plain list.
    pub lead: String,
    /// Whether `lead` is drawn in the accent, as a checked box is.
    pub marked: bool,
    /// Drawn hard right on the row, after the text: an age, a count, a directory.
    pub trail: String,
    /// Fixed-width columns before the text: how long ago, how many times, how big.
    ///
    /// Right-aligned as a block and sized across the whole list, so they form columns down the
    /// screen even though the text beside them varies wildly in length. That alignment is the
    /// entire reason to have them — the eye can scan one column without reading the others.
    pub meta: Vec<String>,
    /// This row's own foreground, when it is not the selected one — a directory in a file list, a
    /// failed job in a job list. The look still owns the background, so a tinted row is still
    /// striped and still highlights when you land on it.
    pub tint: Option<Style>,
}

impl Row {
    /// A row that is only its text.
    pub fn new(text: impl Into<String>) -> Row {
        Row {
            text: text.into(),
            lead: String::new(),
            marked: false,
            trail: String::new(),
            meta: Vec::new(),
            tint: None,
        }
    }
}

/// One width per metadata column, from every row in the list.
///
/// Capped, because a column is a ruler and a ruler that takes half the screen is not one. Twelve
/// cells holds `999999×` and `2025-01-01` and stops well short of crowding out the text.
fn meta_widths(rows: &[Row]) -> Vec<usize> {
    let columns = rows.iter().map(|r| r.meta.len()).max().unwrap_or(0);
    (0..columns)
        .map(|index| {
            rows.iter()
                .filter_map(|row| row.meta.get(index))
                .map(|field| printed_width(field))
                .max()
                .unwrap_or(0)
                .min(12)
        })
        .collect()
}

/// Right-align `text` in exactly `width` cells, truncating if it does not fit.
///
/// Truncation matters as much as padding: a command run a million times renders `999999×`, and one
/// cell of overflow wraps the row — after which every row below it is a line out of place.
fn pad_left(text: &str, width: usize) -> String {
    let text = truncate_to_width(text, width);
    let used = printed_width(&text);
    format!("{}{}", " ".repeat(width.saturating_sub(used)), text)
}

/// Where the list is, so the renderer does not have to be told twice.
pub struct View<'a> {
    pub selected: usize,
    /// First visible row, so a long list scrolls under a fixed window.
    pub offset: usize,
    /// How many rows of list to draw, blanks included.
    pub height: usize,
    pub query: &'a str,
    /// Rows matching the query — what `{n}` means.
    pub matched: usize,
    /// Rows in the whole list — what `{total}` means.
    pub total: usize,
    /// Rows checked — what `{marked}` means.
    pub marked: usize,
    /// Cells available across. The widget subtracts whatever its chrome takes.
    pub cols: usize,
    /// Whether there is a filter row at all.
    pub filtering: bool,
    /// How long the widget has been open, for the scanner.
    ///
    /// Passed in rather than read from a clock so a frame is a pure function of its input and can
    /// be tested without one — the same reason the finder's own frame takes `now`.
    pub elapsed_ms: u64,
}

impl Look {
    /// The list and its filter, in the order this look puts them.
    ///
    /// Every row begins with `\r\n`, which is what makes the row count exact — see
    /// `super::super::Inline`.
    pub fn frame(&self, rows: &[Row], view: &View<'_>) -> String {
        self.rows(rows, view)
            .iter()
            .map(|row| format!("\r\n\r\x1b[K{row}"))
            .collect()
    }

    /// The same rows, painted but not yet joined into a frame.
    ///
    /// The inline widgets want them separated by `\r\n` with an erase on each; the history finder
    /// draws top-down on a screen it owns and wants them the other way round. Both want the *rows*
    /// to be identical, which is the whole reason this is one function rather than two renderers.
    pub fn rows(&self, rows: &[Row], view: &View<'_>) -> Vec<String> {
        let depth = theme::depth();
        // Measured across the *whole* list rather than the visible window, so the columns do not
        // shift under you as it scrolls — which is the one thing that would undo the alignment
        // they exist for.
        let meta = meta_widths(rows);
        let mut list: Vec<String> = (0..view.height)
            .map(|at| match rows.get(view.offset + at) {
                Some(row) => self.list_row(row, view.offset + at, view, &meta, depth),
                // A blank row still takes the surface it would have had, so a half-empty list is a
                // block with space in it rather than a block that stops early.
                None => self.blank(view),
            })
            .collect();
        // Drawn from the far end: the list grows towards the filter, so the best match is the row
        // nearest the cursor rather than the one furthest from it. The unused rows end up at the
        // top, which is where a list growing upward leaves them.
        if self.reverse {
            list.reverse();
        }

        let gap = vec![self.blank(view); self.gap];
        let filter = match view.filtering {
            true => self.filter_rows(view, depth),
            false => Vec::new(),
        };
        match (view.filtering, self.filter_at) {
            (false, _) => list,
            (true, Where::Top) => [filter, gap, list].concat(),
            (true, Where::Bottom) => [list, gap, filter].concat(),
        }
    }

    /// One row: the marker, the lead, the meta columns, the text with its matches marked, and the
    /// trail.
    fn list_row(
        &self,
        row: &Row,
        at: usize,
        view: &View<'_>,
        meta_widths: &[usize],
        depth: theme::Depth,
    ) -> String {
        let here = at == view.selected;
        // The stripe is a property of the row's place in the *list*, so it is the absolute index
        // that decides it. Using the visible one would make the stripes crawl as the list scrolls.
        // A row's own colour is a foreground only, and only when it is not selected: the selection
        // is the stronger statement and a tint that survived it would leave two rows looking picked.
        let plain = match row.tint {
            Some(tint) => Style {
                bg: self.row.bg,
                ..tint
            },
            None => self.row,
        };
        let base = match (here, self.stripe.filter(|_| at % 2 == 1)) {
            (true, _) => self.selected,
            (false, Some(stripe)) => Style {
                bg: stripe.bg.or(plain.bg),
                ..plain
            },
            (false, None) => plain,
        };
        let on = |style: Style| Style {
            bg: base.bg.or(style.bg),
            ..style
        };

        let pad = " ".repeat(self.pad);
        let marker_cells = printed_width(&self.marker);
        let marker = match here {
            true => on(self.accent).paint(&self.marker, depth),
            false => on(Style::default()).paint(&" ".repeat(marker_cells), depth),
        };
        let lead = match row.marked {
            true => on(self.accent).paint(&row.lead, depth),
            false => on(self.muted).paint(&row.lead, depth),
        };

        // Right-aligned in their measured columns, then one space before the text. Painted on the
        // row's own background like everything else: a metadata column that kept the terminal
        // background would punch a hole through a striped row.
        let mut meta = String::new();
        let mut meta_cells = 0usize;
        for (index, width) in meta_widths.iter().enumerate() {
            let field = row.meta.get(index).map(String::as_str).unwrap_or("");
            let cell = format!("{} ", pad_left(field, *width));
            meta_cells += width + 1;
            meta.push_str(&on(self.meta_style).paint(&cell, depth));
        }

        let trail = Look::fill(&row.trail, view);
        let used = self.pad * 2
            + marker_cells
            + printed_width(&row.lead)
            + meta_cells
            + printed_width(&trail);
        let room = view.cols.saturating_sub(used).max(1);
        let shown = truncate_to_width(&row.text, room);
        // Padded before painting, so the background reaches the trail rather than stopping at the
        // last letter — which is the whole point of a full-width row.
        let text = match self.width {
            Width::Full => pad_to_width(&shown, room),
            Width::Content => shown.clone(),
        };

        format!(
            "{}{}{}{}{}{}{}",
            on(Style::default()).paint(&pad, depth),
            marker,
            lead,
            meta,
            self.hits(&text, &shown, view.query, on(base), depth),
            on(self.muted).paint(&trail, depth),
            on(Style::default()).paint(&pad, depth),
        )
    }

    /// A row with nothing on it, still wearing whatever the list is painted on.
    fn blank(&self, view: &View<'_>) -> String {
        match self.width {
            Width::Full => self
                .row
                .paint(&" ".repeat(view.cols), theme::depth())
                .to_string(),
            Width::Content => String::new(),
        }
    }

    /// Paint `padded`, marking the characters `query` matched.
    ///
    /// Runs are painted together, so a contiguous match costs one escape rather than one per
    /// character — which matters on a list redrawn per keystroke.
    fn hits(
        &self,
        padded: &str,
        shown: &str,
        query: &str,
        base: Style,
        depth: theme::Depth,
    ) -> String {
        let marks = crate::ui::matching::positions(shown, query.trim());
        if marks.is_empty() {
            return base.paint(padded, depth);
        }
        let mut out = String::new();
        let mut run = String::new();
        for (at, ch) in padded.char_indices() {
            if !marks.contains(&at) {
                run.push(ch);
                continue;
            }
            if !run.is_empty() {
                out.push_str(&base.paint(&run, depth));
                run.clear();
            }
            out.push_str(&self.hit.paint(&ch.to_string(), depth));
        }
        if !run.is_empty() {
            out.push_str(&base.paint(&run, depth));
        }
        out
    }
}

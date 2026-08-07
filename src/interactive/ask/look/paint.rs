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
use crate::interactive::dropdown::width::{pad_to_width, truncate_to_width};
use crate::interactive::prompt::printed_width;
use crate::interactive::theme::{self, Color, Style};

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
            tint: None,
        }
    }
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
}

impl Look {
    /// The list and its filter, in the order this look puts them.
    ///
    /// Every row begins with `\r\n`, which is what makes the row count exact — see
    /// `super::super::Inline`.
    pub fn frame(&self, rows: &[Row], view: &View<'_>) -> String {
        let depth = theme::depth();
        let mut list: Vec<String> = (0..view.height)
            .map(|at| match rows.get(view.offset + at) {
                Some(row) => self.list_row(row, view.offset + at, view, depth),
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
        let ordered: Vec<String> = match (view.filtering, self.filter_at) {
            (false, _) => list,
            (true, Where::Top) => [filter, gap, list].concat(),
            (true, Where::Bottom) => [list, gap, filter].concat(),
        };
        ordered
            .iter()
            .map(|row| format!("\r\n\r\x1b[K{row}"))
            .collect()
    }

    /// A widget that is one row rather than a list: `input`, and anything else that asks for a
    /// line rather than a choice.
    ///
    /// Shares the surface with a filter row, which is the point — `ui input --surface 236
    /// --surface-rows 3` and `ui filter --surface 236 --surface-rows 3` are the same panel, so a
    /// script can put a form together out of both without them disagreeing about what a surface
    /// looks like. `body` is already painted and already carries its own caret.
    pub fn one_row(&self, prompt: &str, body: &str, cols: usize) -> String {
        let depth = theme::depth();
        let on = |style: Style| Style {
            bg: self.surface.or(style.bg),
            ..style
        };
        let pad = " ".repeat(self.pad);
        let used = self.pad * 2 + printed_width(prompt) + printed_width(body);
        let fill = match self.width {
            Width::Full => cols.saturating_sub(used),
            Width::Content => 0,
        };
        let row = format!(
            "{}{}{}{}{}",
            on(Style::default()).paint(&pad, depth),
            on(self.accent).paint(prompt, depth),
            body,
            on(Style::default()).paint(&" ".repeat(fill), depth),
            on(Style::default()).paint(&pad, depth),
        );
        let blank = on(Style::default()).paint(&" ".repeat(cols), depth);
        let middle = self.surface_rows / 2;
        (0..self.surface_rows.max(1))
            .map(|at| match at == middle {
                true => row.clone(),
                false => blank.clone(),
            })
            .map(|row| format!("\r\x1b[K{row}"))
            .collect::<Vec<_>>()
            .join("\r\n")
    }

    /// One row: the marker, the lead, the text with its matches marked, and the trail.
    fn list_row(&self, row: &Row, at: usize, view: &View<'_>, depth: theme::Depth) -> String {
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

        let trail = Look::slot(&row.trail, view);
        let used = self.pad * 2 + marker_cells + printed_width(&row.lead) + printed_width(&trail);
        let room = view.cols.saturating_sub(used).max(1);
        let shown = truncate_to_width(&row.text, room);
        // Padded before painting, so the background reaches the trail rather than stopping at the
        // last letter — which is the whole point of a full-width row.
        let text = match self.width {
            Width::Full => pad_to_width(&shown, room),
            Width::Content => shown.clone(),
        };

        format!(
            "{}{}{}{}{}{}",
            on(Style::default()).paint(&pad, depth),
            marker,
            lead,
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
        let marks = crate::interactive::matching::positions(shown, query.trim());
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

    /// The filter: its surface, the query, and whatever the slots say.
    fn filter_rows(&self, view: &View<'_>, depth: theme::Depth) -> Vec<String> {
        let on = |style: Style| Style {
            bg: self.surface.or(style.bg),
            ..style
        };
        let blank = on(Style::default()).paint(&" ".repeat(view.cols), depth);
        // The query goes in the middle of the surface, which is what makes three rows read as a
        // panel with something in it rather than as a line with two spare rows.
        let middle = self.surface_rows / 2;
        (0..self.surface_rows.max(1))
            .map(|row| match row == middle {
                true => self.query_row(view, depth),
                false => blank.clone(),
            })
            .collect()
    }

    /// The row the query is on: prompt, what has been typed, a caret, then the slots.
    fn query_row(&self, view: &View<'_>, depth: theme::Depth) -> String {
        let on = |style: Style| Style {
            bg: self.surface.or(style.bg),
            ..style
        };
        let caret = Style {
            fg: Some(Color::Indexed(0)),
            bg: Some(Color::Indexed(1)),
            ..Style::default()
        };

        let left = Look::slot(&self.left, view);
        let right = Look::slot(&self.right, view);
        let fixed = self.pad * 2
            + printed_width(&self.prompt)
            + printed_width(&left)
            + printed_width(&right)
            // The caret is part of the input and has to fit.
            + 1;
        let room = view.cols.saturating_sub(fixed).max(1);

        let (typed, style) = match view.query.is_empty() {
            true => (truncate_to_width(&self.placeholder, room), self.muted),
            false => (truncate_to_width(view.query, room), self.row),
        };
        let gap = match self.width {
            Width::Full => room.saturating_sub(printed_width(&typed)),
            Width::Content => 0,
        };
        let pad = " ".repeat(self.pad);

        format!(
            "{}{}{}{}{}{}{}{}",
            on(Style::default()).paint(&pad, depth),
            on(self.accent).paint(&self.prompt, depth),
            on(self.muted).paint(&left, depth),
            on(style).paint(&typed, depth),
            // **A drawn caret, not the real one.** The terminal cursor is hidden while a widget is
            // open — an inline one repaints its whole block per keystroke and would drag the
            // cursor across all of it — so the caret is part of the frame and cannot end up a cell
            // out of step with the text.
            caret.paint(" ", depth),
            on(Style::default()).paint(&" ".repeat(gap), depth),
            on(self.muted).paint(&right, depth),
            on(Style::default()).paint(&pad, depth),
        )
    }
}

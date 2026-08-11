//! The filter row: the scanner, the prompt, what has been typed, and the slots either side.
//!
//! Split from [`super::paint`] because it is a different problem. A list row is one thing repeated
//! with small differences; this is one row made of six parts that all have to add up to the width
//! available — and getting that arithmetic wrong is how a bar comes out a cell short or wraps.
//!
//! # The bar reads left to right
//!
//! ```text
//!  ⬝⬝⬝⬝⬝⬝⬝⬝⬝  >>  cargo t▌                    profile @ [global] || 12/840
//!  └ scanner   └ prompt └ query               └ left slot, badge, right slot
//! ```
//!
//! The scanner says the widget is live, the badge is the only part with a background because it is
//! the only part that is a *state you can change*, and the slots are facts about what you are
//! looking at. Everything between them is painted, gaps included: a panel with a hole in it is not
//! a panel.

use super::{Look, View, Width};
use crate::dropdown::width::truncate_to_width;
use crate::prompt::printed_width;
use crate::theme::{self, Style};

impl Look {
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
        self.surfaced(&row, cols, depth)
    }

    /// The filter: its surface, the query, and whatever the slots say.
    pub(super) fn filter_rows(&self, view: &View<'_>, depth: theme::Depth) -> Vec<String> {
        let row = self.query_row(view, depth);
        self.surfaced(&row, view.cols, depth)
            .split("\r\n")
            .map(|row| row.trim_start_matches('\r').replace("\x1b[K", ""))
            .collect()
    }

    /// `row` in the middle of however many rows of surface this look asks for.
    ///
    /// The blank rows *are* the surface, not spacing around it — they carry the same colour, which
    /// is what makes three rows read as somewhere to type rather than as a line with two spare
    /// rows above and below it.
    fn surfaced(&self, row: &str, cols: usize, depth: theme::Depth) -> String {
        let on = |style: Style| Style {
            bg: self.surface.or(style.bg),
            ..style
        };
        // **The blank rows are as wide as the row they wrap, not as wide as the terminal.** A
        // surface that reached the edge while the query row stopped at its text drew a panel in
        // two different widths — the tint running to the margin above and below a line that did
        // not. `Width` says which the caller wanted; it now says it about the whole panel.
        let across = match self.width {
            Width::Full => cols,
            Width::Content => printed_width(row).max(self.min_width).min(cols),
        };
        let blank = on(Style::default()).paint(&" ".repeat(across), depth);
        let middle = self.surface_rows / 2;
        (0..self.surface_rows.max(1))
            .map(|at| match at == middle {
                true => row.to_string(),
                false => blank.clone(),
            })
            .map(|row| format!("\r\x1b[K{row}"))
            .collect::<Vec<_>>()
            .join("\r\n")
    }

    /// The row the query is on: the sweep, the prompt, what has been typed, and the slots.
    fn query_row(&self, view: &View<'_>, depth: theme::Depth) -> String {
        let on = |style: Style| Style {
            bg: self.surface.or(style.bg),
            ..style
        };
        // Measured from `plain`, not from the painted sweep: the rendered one is mostly escapes,
        // and counting those as cells is how the right-hand slot ends up off the edge.
        let (sweep, sweep_cells) = match self.scanner {
            Some(scanner) => (
                scanner.render(view.elapsed_ms, self.surface, depth),
                scanner.plain(view.elapsed_ms).chars().count(),
            ),
            None => (String::new(), 0),
        };
        let (left, left_cells) = self.slot(&self.left, view, depth);
        let (right, right_cells) = self.slot(&self.right, view, depth);

        let fixed = self.pad * 2
            + sweep_cells
            + printed_width(&self.prompt)
            + left_cells
            + right_cells
            // One cell for the caret, which is part of the input and has to fit.
            + 1;
        let room = view.cols.saturating_sub(fixed).max(1);

        // **The caret is where you are typing, which on an empty field is the beginning.** It sat
        // after the placeholder, so an untouched filter looked like a cursor parked at the end of
        // the words "type to filter" rather than in front of them, waiting.
        //
        // Drawn by the shared `with_caret`, so it takes the shape `oslo.vi.cursor_insert` asks
        // for — a widget with its own hard-coded block left the shell with two different cursors
        // depending on which one you were in.
        let (body, cells) = match view.query.is_empty() {
            true => {
                let shown = truncate_to_width(&self.placeholder, room);
                let mut chars = shown.chars();
                let first = chars.next().unwrap_or(' ');
                (
                    format!(
                        "{}{}",
                        super::super::with_caret_on(&first.to_string(), 0, self.surface),
                        on(self.muted).paint(chars.as_str(), depth)
                    ),
                    printed_width(&shown).max(1),
                )
            }
            false => {
                let shown = truncate_to_width(view.query, room);
                let cells = printed_width(&shown) + 1;
                (
                    format!(
                        "{}{}",
                        on(self.row).paint(&shown, depth),
                        // Past the last character, so it reads as "typing continues here".
                        super::super::with_caret_on("", 0, self.surface)
                    ),
                    cells,
                )
            }
        };
        // The query row is padded to the floor as well, or the tint above and below it would reach
        // further than the row between them.
        let gap = match self.width {
            Width::Full => (room + 1).saturating_sub(cells),
            Width::Content => self
                .min_width
                .saturating_sub(fixed + cells)
                .min(room.saturating_sub(cells)),
        };
        let pad = " ".repeat(self.pad);

        format!(
            "{}{}{}{}{}{}{}{}",
            on(Style::default()).paint(&pad, depth),
            sweep,
            on(self.accent).paint(&self.prompt, depth),
            left,
            body,
            on(Style::default()).paint(&" ".repeat(gap), depth),
            right,
            on(Style::default()).paint(&pad, depth),
        )
    }

    /// A slot template, painted, with its printed width.
    ///
    /// The width comes back with it because the caller cannot recover it afterwards: the string is
    /// mostly escapes by then, and a slot with a badge in it carries two styles. Measuring at the
    /// point of building is the only place the answer is cheap and certain.
    fn slot(&self, template: &str, view: &View<'_>, depth: theme::Depth) -> (String, usize) {
        if template.is_empty() {
            return (String::new(), 0);
        }
        let on = |style: Style| Style {
            bg: self.surface.or(style.bg),
            ..style
        };
        let filled = Look::fill(template, view);
        // No badge asked for, or nowhere to put it: one style over the whole slot.
        if self.badge.is_empty() || !filled.contains("{badge}") {
            let text = filled.replace("{badge}", "");
            let cells = printed_width(&text);
            return (on(self.muted).paint(&text, depth), cells);
        }
        let badge = Look::fill(&self.badge, view);
        let mut out = String::new();
        let mut cells = 0usize;
        for (at, part) in filled.split("{badge}").enumerate() {
            if at > 0 {
                out.push_str(&self.badge_style.paint(&badge, depth));
                cells += printed_width(&badge);
            }
            out.push_str(&on(self.muted).paint(part, depth));
            cells += printed_width(part);
        }
        (out, cells)
    }
}

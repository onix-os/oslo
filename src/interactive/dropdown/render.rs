//! Painting the dropdown: one string of escapes, plus the number of rows to walk back up.
//!
//! The row count is the load-bearing half. Whatever is returned here is what the selection loop
//! feeds to `ESC [ n A`, so it must be counted in *physical* rows — the rows the terminal really
//! used after wrapping — or the cursor lands above the prompt and the redraw eats the scrollback.

use super::CompletionCandidate;
use super::layout::compute_layout;
use super::width::{
    FALLBACK_COLS, display_width, pad_to_width, physical_rows, terminal_cols, truncate_to_width,
};

/// The most candidates shown at once; beyond this the list pages.
pub const MAX_ROWS: usize = 8;

/// Render the dropdown against the real terminal's width.
///
/// Returns the escape-laden string to write and the number of **physical** rows it occupies —
/// the count to feed to `ESC [ n A` afterwards.
pub fn render_vertical_dropdown(
    candidates: &[CompletionCandidate],
    selected_idx: usize,
    max_visible: usize,
    indent_cols: usize,
) -> (String, usize) {
    render_vertical_dropdown_at_width(
        candidates,
        selected_idx,
        max_visible,
        indent_cols,
        terminal_cols(),
    )
}

/// Render the dropdown into a `cols`-wide terminal.
///
/// `cols` is a parameter rather than a query so the layout can be tested at a fixed width with no
/// terminal attached; [`render_vertical_dropdown`] is the one that asks the kernel.
pub fn render_vertical_dropdown_at_width(
    candidates: &[CompletionCandidate],
    selected_idx: usize,
    max_visible: usize,
    indent_cols: usize,
    cols: usize,
) -> (String, usize) {
    if candidates.is_empty() {
        return (String::new(), 0);
    }

    let max_visible = max_visible.clamp(1, MAX_ROWS);
    let start = (selected_idx / max_visible) * max_visible;
    let end = (start + max_visible).min(candidates.len());
    let visible = &candidates[start..end];

    let cols = if cols == 0 { FALLBACK_COLS } else { cols };
    let layout = compute_layout(visible, indent_cols, cols);
    let indent = " ".repeat(layout.indent);

    let mut out = String::new();

    out.push_str("\r\n");
    let title = format!(" Suggestions ({}/{}) ", selected_idx + 1, candidates.len());
    let counter = format!(" ({}/{}) ", selected_idx + 1, candidates.len());
    out.push_str(&border_row(&indent, &title, &counter, layout.box_w, true));

    for (i, cand) in visible.iter().enumerate() {
        let selected = start + i == selected_idx;
        let label = pad_to_width(
            &truncate_to_width(&format!("{}{}", cand.icon(), cand.display), layout.label_w),
            layout.label_w,
        );

        out.push_str("\r\n");
        out.push_str(&indent);
        out.push_str("\x1b[38;5;240m│\x1b[0m ");
        // The selected row takes the indigo highlight, the rest a dark slate.
        out.push_str(if selected {
            "\x1b[48;5;62m\x1b[1;97m ▶ "
        } else {
            "\x1b[48;5;236m\x1b[37m   "
        });
        out.push_str(&label);
        if layout.has_desc() {
            let desc = pad_to_width(
                &truncate_to_width(cand.description.as_deref().unwrap_or(""), layout.desc_w),
                layout.desc_w,
            );
            out.push_str(if selected {
                " \x1b[36m "
            } else {
                " \x1b[38;5;245m "
            });
            out.push_str(&desc);
        }
        out.push_str(" \x1b[0m \x1b[38;5;240m│\x1b[0m\x1b[K");
    }

    out.push_str("\r\n");
    out.push_str(&border_row(
        &indent,
        " Tab/Enter to select ",
        " Tab ",
        layout.box_w,
        false,
    ));

    // Every row is exactly `row_width()` wide, so one wrap count covers all of them.
    let rows = visible.len() + 2;
    (out, rows * physical_rows(layout.row_width(), cols))
}

/// Top or bottom border, exactly `box_w` cells wide including both corners, with `text` inlaid
/// after the left corner.
///
/// A box too narrow for `text` falls back to `short` — an intact `(3/12)` tells you more than a
/// chopped `Suggestions (3…` — and only then to a hard truncation.
fn border_row(indent: &str, text: &str, short: &str, box_w: usize, top: bool) -> String {
    let (left, right, colour) = if top {
        ("╭─", "╮", "\x1b[1;36m")
    } else {
        ("╰─", "╯", "\x1b[90m")
    };
    // `╭─` is 2 cells and the closing corner 1.
    let budget = box_w.saturating_sub(3);
    let text = if display_width(text) <= budget {
        text.to_string()
    } else if display_width(short) <= budget {
        short.to_string()
    } else {
        truncate_to_width(text, budget)
    };
    let fill = budget - display_width(&text);
    format!(
        "{}\x1b[38;5;240m{}{}{}\x1b[0m\x1b[38;5;240m{}{}\x1b[0m\x1b[K",
        indent,
        left,
        colour,
        text,
        "─".repeat(fill),
        right
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cand(display: &str, desc: Option<&str>) -> CompletionCandidate {
        CompletionCandidate::new(
            display.to_string(),
            display.to_string(),
            desc.map(str::to_string),
        )
    }

    /// Visible width of every row the renderer emitted, indent included.
    fn row_widths(rendered: &str) -> Vec<usize> {
        rendered
            .split("\r\n")
            .filter(|r| !r.is_empty())
            .map(display_width)
            .collect()
    }

    #[test]
    fn no_row_exceeds_eighty_columns_with_a_deep_indent() {
        // R9.4 as measured: a deep cwd pushed the indent far right and rows reached ~297 cells.
        let candidates: Vec<_> = (0..6)
            .map(|i| {
                cand(
                    &format!("some-quite-long-completion-candidate-number-{i}"),
                    Some("a description long enough to blow past the right margin on its own"),
                )
            })
            .collect();
        let (rendered, lines) = render_vertical_dropdown_at_width(&candidates, 0, 8, 65, 80);
        for w in row_widths(&rendered) {
            assert!(w <= 80, "row of {w} cells overflows an 80-column terminal");
        }
        // Nothing wrapped, so the cursor walks back exactly the rows that were drawn.
        assert_eq!(lines, 8);
    }

    #[test]
    fn every_row_is_the_same_width_so_the_box_lines_up() {
        let candidates = vec![
            cand("cargo", Some("Rust package manager")),
            cand("cd", Some("Change working directory")),
            cand("chmod", None),
        ];
        let (rendered, _) = render_vertical_dropdown_at_width(&candidates, 0, 8, 4, 80);
        let widths = row_widths(&rendered);
        assert_eq!(widths.len(), 5);
        assert!(
            widths.iter().all(|w| *w == widths[0]),
            "border and item rows disagree: {widths:?}"
        );
    }

    #[test]
    fn an_indent_wider_than_the_screen_does_not_push_rows_off_it() {
        let candidates = vec![cand("cargo", Some("Rust package manager")); 3];
        let (rendered, lines) = render_vertical_dropdown_at_width(&candidates, 0, 8, 200, 80);
        for w in row_widths(&rendered) {
            assert!(
                w <= 80,
                "row of {w} cells overflows: indent was not clamped"
            );
        }
        assert_eq!(lines, 5);
    }

    #[test]
    fn long_labels_and_descriptions_are_ellipsised() {
        let candidates = vec![cand(
            "an-extremely-long-candidate-name-that-cannot-possibly-fit-in-any-box",
            Some("and a description that is likewise far too long to display in full"),
        )];
        let (rendered, _) = render_vertical_dropdown_at_width(&candidates, 0, 8, 0, 80);
        assert!(rendered.contains('…'), "nothing was ellipsised: {rendered}");
        for w in row_widths(&rendered) {
            assert!(w <= 80);
        }
    }

    #[test]
    fn a_narrow_terminal_still_produces_rows_that_fit() {
        let candidates = vec![
            cand("cargo", Some("Rust package manager")),
            cand("cd", Some("Change working directory")),
        ];
        for cols in [20usize, 30, 40, 60, 100, 200] {
            let (rendered, lines) = render_vertical_dropdown_at_width(&candidates, 0, 8, 12, cols);
            for w in row_widths(&rendered) {
                assert!(w <= cols, "row of {w} cells at {cols} columns");
            }
            assert_eq!(lines, 4, "at {cols} columns");
        }
    }

    #[test]
    fn wrapped_rows_are_counted_as_the_physical_rows_they_occupy() {
        // Below the width any box fits in, the rows do wrap — and the count must say so, or the
        // cursor-up walks too few rows and paints over the prompt.
        let candidates = vec![cand("cargo", None); 2];
        let (rendered, lines) = render_vertical_dropdown_at_width(&candidates, 0, 8, 0, 6);
        let widths = row_widths(&rendered);
        let expected: usize = widths.iter().map(|w| physical_rows(*w, 6)).sum();
        assert_eq!(lines, expected);
        assert!(lines > widths.len(), "wrapping was not accounted for");
    }

    #[test]
    fn the_header_keeps_the_counter_when_the_title_will_not_fit() {
        let candidates: Vec<_> = (0..20).map(|i| cand(&format!("cmd{i}"), None)).collect();
        let (rendered, lines) = render_vertical_dropdown_at_width(&candidates, 9, 8, 0, 80);
        // Selection 9 is on the second page: cmd8..=cmd15.
        assert!(rendered.contains("cmd9"));
        assert!(!rendered.contains("cmd0 "));
        assert_eq!(lines, 10);
        assert!(rendered.contains("(10/20)"), "counter was lost: {rendered}");
    }

    #[test]
    fn empty_candidate_list_renders_nothing() {
        let (rendered, lines) = render_vertical_dropdown_at_width(&[], 0, 8, 0, 80);
        assert!(rendered.is_empty());
        assert_eq!(lines, 0);
    }

    #[test]
    fn a_zero_max_visible_still_shows_one_row() {
        let candidates = vec![cand("cargo", None); 3];
        let (rendered, lines) = render_vertical_dropdown_at_width(&candidates, 0, 0, 0, 80);
        assert_eq!(lines, 3);
        assert!(rendered.contains("cargo"));
    }
}

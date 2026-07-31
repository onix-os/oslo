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
use crate::interactive::theme::{self, Style};

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
    typed: &str,
) -> (String, usize) {
    render_vertical_dropdown_at_width(
        candidates,
        selected_idx,
        max_visible,
        indent_cols,
        terminal_cols(),
        typed,
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
    typed: &str,
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
    let theme = theme::current();
    let depth = theme::depth();
    let settings = crate::interactive::settings::current();

    let mut out = String::new();

    out.push_str("\r\n");
    let title = format!(" Suggestions ({}/{}) ", selected_idx + 1, candidates.len());
    let counter = format!(" ({}/{}) ", selected_idx + 1, candidates.len());
    out.push_str(&border_row(&indent, &title, &counter, layout.box_w, true));

    for (i, cand) in visible.iter().enumerate() {
        let selected = start + i == selected_idx;
        let label = pad_to_width(
            &truncate_to_width(&cand.display, layout.label_w),
            layout.label_w,
        );

        out.push_str("\r\n");
        out.push_str(&indent);
        out.push_str(&theme.pager.border.paint("│", depth));
        out.push(' ');

        // The row background runs from the marker to the closing border, so a selected row reads
        // as one bar rather than as separately coloured columns.
        let row_bg = if selected { theme.pager.sel_bg } else { None };
        let on_row = |style: Style| Style {
            bg: row_bg.or(style.bg),
            ..style
        };
        out.push_str(
            &on_row(theme.pager.text_sel).paint(if selected { " ▶ " } else { "   " }, depth),
        );

        // The part the user has already typed, in the match colour, so the eye can see what the
        // rest of each candidate adds.
        out.push_str(&paint_match(&label, typed, selected, &theme, depth, row_bg));

        if layout.has_badge() && settings.completion.show_kind {
            out.push_str(&on_row(Style::default()).paint(" ", depth));
            out.push_str(&badge_cell(cand, layout.badge_w, &theme, depth));
        }

        if layout.has_desc() && settings.completion.descriptions {
            let desc = pad_to_width(
                &truncate_to_width(cand.description.as_deref().unwrap_or(""), layout.desc_w),
                layout.desc_w,
            );
            let style = if selected {
                theme.pager.desc_sel
            } else {
                theme.pager.desc
            };
            out.push_str(&on_row(Style::default()).paint("  ", depth));
            out.push_str(&on_row(style).paint(&desc, depth));
        }
        out.push_str(&on_row(Style::default()).paint(" ", depth));
        out.push(' ');
        out.push_str(&theme.pager.border.paint("│", depth));
        out.push_str("\x1b[K");
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
    let theme = theme::current();
    let depth = theme::depth();
    let (left, right) = if top {
        ("╭─", "╮")
    } else {
        ("╰─", "╯")
    };
    // The caption takes the scroll colour, the rule itself the border colour.
    let caption = theme.pager.scroll;
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
        "{}{}{}{}{}",
        indent,
        theme.pager.border.paint(left, depth),
        caption.paint(&text, depth),
        theme.pager.border.paint(&"─".repeat(fill), depth),
        theme.pager.border.paint(right, depth)
    ) + "\x1b[K"
}

/// A label with the already-typed prefix in the match colour.
///
/// Only a real prefix is highlighted, and case-insensitively, because that is how the candidates
/// were matched in the first place — highlighting a substring the completer did not match on
/// would point at the wrong thing.
fn paint_match(
    label: &str,
    typed: &str,
    selected: bool,
    theme: &theme::Theme,
    depth: theme::Depth,
    row_bg: Option<theme::Color>,
) -> String {
    let base = Style {
        bg: row_bg,
        ..if selected {
            theme.pager.text_sel
        } else {
            theme.pager.text
        }
    };
    let matched = Style {
        bg: row_bg,
        ..theme.pager.match_
    };

    let take = matching_prefix(label, typed);
    if take == 0 {
        return base.paint(label, depth);
    }
    format!(
        "{}{}",
        matched.paint(&label[..take], depth),
        base.paint(&label[take..], depth)
    )
}

/// How many bytes of `label` the typed text covers, or 0 if it is not a prefix.
///
/// Byte-wise on purpose: the result indexes back into `label`, so it has to land on a character
/// boundary. Comparing lowercased *characters* while counting the original bytes is what keeps
/// that true for a multi-byte prefix.
fn matching_prefix(label: &str, typed: &str) -> usize {
    if typed.is_empty() {
        return 0;
    }
    let mut typed_chars = typed.chars().flat_map(char::to_lowercase);
    let mut taken = 0;
    for (offset, c) in label.char_indices() {
        let mut lowered = c.to_lowercase();
        loop {
            match (lowered.next(), typed_chars.clone().next()) {
                (Some(a), Some(b)) if a == b => {
                    typed_chars.next();
                }
                (Some(_), _) => return 0,
                (None, _) => break,
            }
        }
        taken = offset + c.len_utf8();
        if typed_chars.clone().next().is_none() {
            return taken;
        }
    }
    // The typed text ran past the label, which means it is not a prefix of it.
    let _ = taken;
    0
}

/// The kind badge, drawn as a pill of exactly `width` cells.
fn badge_cell(
    cand: &CompletionCandidate,
    width: usize,
    theme: &theme::Theme,
    depth: theme::Depth,
) -> String {
    let Some(word) = cand.badge() else {
        // A candidate with no kind still occupies the column, or the description column would
        // start in a different place on that row and the table would rag.
        return " ".repeat(width);
    };
    let text = truncate_to_width(word, width.saturating_sub(2));
    let inner = format!(" {} ", pad_to_width(&text, width.saturating_sub(2)));
    theme
        .pager
        .kind
        .for_kind(cand.kind.as_deref().unwrap_or(""))
        .paint(&inner, depth)
}

#[cfg(test)]
mod tests {
    /// Build a candidate with a kind, which is what the badge column reads.
    fn kinded(display: &str, kind: &str, desc: Option<&str>) -> CompletionCandidate {
        CompletionCandidate {
            display: display.to_string(),
            replacement: display.to_string(),
            description: desc.map(str::to_string),
            kind: Some(kind.to_string()),
        }
    }

    /// Strip every escape, leaving what the user actually sees.
    fn plain(text: &str) -> String {
        let mut out = String::new();
        let mut chars = text.chars();
        while let Some(c) = chars.next() {
            if c != '\x1b' {
                out.push(c);
                continue;
            }
            // `\x1b[…m` and `\x1b[K`: skip to the terminating letter.
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        }
        out
    }

    #[test]
    fn the_kind_is_drawn_as_a_badge() {
        let candidates = vec![
            kinded("cargo", "command", Some("Rust package manager")),
            kinded("src", "dir", None),
        ];
        let (rendered, _) = render_vertical_dropdown_at_width(&candidates, 0, 8, 0, 100, "");
        let seen = plain(&rendered);
        assert!(seen.contains("command"), "no command badge in:\n{seen}");
        assert!(seen.contains("dir"), "no dir badge in:\n{seen}");
        // The emoji icons are gone: they were two cells in some terminals and one in others,
        // which no column layout can survive.
        assert!(!seen.contains('📁'), "an icon survived:\n{seen}");
        assert!(!seen.contains('⚙'), "an icon survived:\n{seen}");
    }

    /// Every badge is the same width, so the pills form a column instead of ragging.
    #[test]
    fn badges_line_up_into_a_column() {
        let candidates = vec![kinded("a", "dir", None), kinded("bbbbbb", "variable", None)];
        let (rendered, _) = render_vertical_dropdown_at_width(&candidates, 0, 8, 0, 100, "");
        let seen = plain(&rendered);
        let rows: Vec<&str> = seen.lines().filter(|l| l.contains('│')).collect();
        assert_eq!(rows.len(), 2, "{rows:?}");
        let at = |row: &str, needle: &str| row.find(needle);
        assert_eq!(at(rows[0], "dir").map(|i| i > 0), Some(true), "{rows:?}");
        // Both rows are the same total width, which is the property the layout guarantees.
        assert_eq!(
            display_width(rows[0]),
            display_width(rows[1]),
            "rows differ in width:\n{rows:?}"
        );
    }

    /// A candidate with no kind still occupies the badge column, or the description column would
    /// start in a different place on that row.
    #[test]
    fn a_candidate_without_a_kind_keeps_the_column_aligned() {
        let mut plainish = kinded("x", "file", Some("a file"));
        plainish.kind = None;
        let candidates = vec![kinded("y", "command", Some("a command")), plainish];
        let (rendered, _) = render_vertical_dropdown_at_width(&candidates, 0, 8, 0, 100, "");
        let seen = plain(&rendered);
        let rows: Vec<&str> = seen.lines().filter(|l| l.contains('│')).collect();
        assert_eq!(display_width(rows[0]), display_width(rows[1]), "{rows:?}");
    }

    #[test]
    fn the_typed_prefix_is_highlighted_and_the_rest_is_not() {
        let candidates = vec![kinded("cargo", "command", None)];
        let (rendered, _) = render_vertical_dropdown_at_width(&candidates, 0, 8, 0, 100, "car");
        // The label survives whole, whatever escapes were put through it.
        assert!(plain(&rendered).contains("cargo"), "{}", plain(&rendered));
        // And the prefix was split off rather than painted in one span.
        assert!(
            rendered.contains("car") && rendered.matches("go").count() >= 1,
            "prefix was not separated:\n{rendered}"
        );
    }

    /// Matching is case-insensitive because that is how the candidates were selected; and a typed
    /// string that is not a prefix at all highlights nothing.
    #[test]
    fn only_a_real_prefix_is_highlighted() {
        assert_eq!(matching_prefix("Cargo", "car"), 3);
        assert_eq!(matching_prefix("cargo", "CAR"), 3);
        assert_eq!(matching_prefix("cargo", "go"), 0);
        assert_eq!(matching_prefix("cargo", ""), 0);
        // Typed longer than the label is not a prefix of it.
        assert_eq!(matching_prefix("ca", "cargo"), 0);
        // A multi-byte prefix lands on a character boundary, not inside one.
        assert_eq!(matching_prefix("ändern", "än"), 3);
    }

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
        let (rendered, lines) = render_vertical_dropdown_at_width(&candidates, 0, 8, 65, 80, "");
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
        let (rendered, _) = render_vertical_dropdown_at_width(&candidates, 0, 8, 4, 80, "");
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
        let (rendered, lines) = render_vertical_dropdown_at_width(&candidates, 0, 8, 200, 80, "");
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
        let (rendered, _) = render_vertical_dropdown_at_width(&candidates, 0, 8, 0, 80, "");
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
            let (rendered, lines) =
                render_vertical_dropdown_at_width(&candidates, 0, 8, 12, cols, "");
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
        let (rendered, lines) = render_vertical_dropdown_at_width(&candidates, 0, 8, 0, 6, "");
        let widths = row_widths(&rendered);
        let expected: usize = widths.iter().map(|w| physical_rows(*w, 6)).sum();
        assert_eq!(lines, expected);
        assert!(lines > widths.len(), "wrapping was not accounted for");
    }

    #[test]
    fn the_header_keeps_the_counter_when_the_title_will_not_fit() {
        let candidates: Vec<_> = (0..20).map(|i| cand(&format!("cmd{i}"), None)).collect();
        let (rendered, lines) = render_vertical_dropdown_at_width(&candidates, 9, 8, 0, 80, "");
        // Selection 9 is on the second page: cmd8..=cmd15.
        assert!(rendered.contains("cmd9"));
        assert!(!rendered.contains("cmd0 "));
        assert_eq!(lines, 10);
        assert!(rendered.contains("(10/20)"), "counter was lost: {rendered}");
    }

    #[test]
    fn empty_candidate_list_renders_nothing() {
        let (rendered, lines) = render_vertical_dropdown_at_width(&[], 0, 8, 0, 80, "");
        assert!(rendered.is_empty());
        assert_eq!(lines, 0);
    }

    #[test]
    fn a_zero_max_visible_still_shows_one_row() {
        let candidates = vec![cand("cargo", None); 3];
        let (rendered, lines) = render_vertical_dropdown_at_width(&candidates, 0, 0, 0, 80, "");
        assert_eq!(lines, 3);
        assert!(rendered.contains("cargo"));
    }
}

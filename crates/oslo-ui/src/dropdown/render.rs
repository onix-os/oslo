//! Painting the dropdown: one string of escapes, plus the number of rows to walk back up.
//!
//! The row count is the load-bearing half. Whatever is returned here is what the selection loop
//! feeds to `ESC [ n A`, so it must be counted in *physical* rows — the rows the terminal really
//! used after wrapping — or the cursor lands above the prompt and the redraw eats the scrollback.

use super::CompletionCandidate;
use super::columns::columns_for_rows;
use super::layout::compute_layout;
use super::width::{FALLBACK_COLS, pad_to_width, physical_rows, terminal_cols, truncate_to_width};
use crate::theme::{self, Style};

/// How many candidates are shown at once when the config does not say; beyond this the list pages.
///
/// A **default**, not a ceiling. It used to be both, which made `oslo.completion.max_rows` a knob
/// that could only ever lower the menu — asking for twenty rows silently got you eight, and the
/// documented default of fifteen was unreachable.
pub const DEFAULT_ROWS: usize = 8;

/// The most rows a menu may take however large `max_rows` is, so a dropdown can never fill the
/// screen and leave nowhere for the prompt.
pub const CEILING_ROWS: usize = 40;

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

    let max_visible = max_visible.clamp(1, CEILING_ROWS);
    let start = (selected_idx / max_visible) * max_visible;
    let end = (start + max_visible).min(candidates.len());
    let visible = &candidates[start..end];

    let cols = if cols == 0 { FALLBACK_COLS } else { cols };
    // Computed here, for the visible slice and nothing else. A size column means a `stat` per row,
    // which is fifteen syscalls a frame — and would be three thousand if it were done when the
    // candidates were collected. See `super::columns`.
    let cells = columns_for_rows(visible);
    let layout = compute_layout(visible, &cells, indent_cols, cols);
    let indent = " ".repeat(layout.indent);
    let theme = theme::current();
    let depth = theme::depth();
    let settings = crate::settings::current();

    let mut out = String::new();

    for (i, cand) in visible.iter().enumerate() {
        let selected = start + i == selected_idx;
        let label = pad_to_width(
            &truncate_to_width(&cand.display, layout.label_w),
            layout.label_w,
        );

        out.push_str("\r\n");
        out.push_str(&indent);

        // The whole row takes a background, which is the only thing marking the menu out now that
        // there is no border around it.
        let row_bg = if selected {
            theme.pager.sel_bg
        } else {
            theme.pager.bg
        };
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
            out.push_str(&badge_cell(cand, layout.badge_w, selected, &theme, depth));
        }

        // The description is the first info column; everything after it is what the kind — or the
        // config — had left to say. They differ only in colour: the description is the loudest of
        // them because it is the one that explains the candidate rather than annotating it.
        // `descriptions` is honoured where the columns are built, not here: it removes the
        // description and lets the columns after it move left, which the layout must already know
        // about by the time it is sizing anything.
        if !layout.columns.is_empty() {
            for (col, width) in layout.columns.iter().copied().enumerate() {
                let text = cells[i].get(col).map(String::as_str).unwrap_or("");
                let text = pad_to_width(&truncate_to_width(text, width), width);
                let style = theme.pager.column(col, selected);
                out.push_str(&on_row(Style::default()).paint("  ", depth));
                out.push_str(&on_row(style).paint(&text, depth));
            }
        }
        out.push_str(&on_row(Style::default()).paint(" ", depth));
        // Cleared to the end of the row so a shorter line does not leave the previous frame's
        // background hanging past it.
        out.push_str("\x1b[K");
    }

    // Every row is exactly `row_width()` wide, so one wrap count covers all of them.
    let rows = visible.len();
    (out, rows * physical_rows(layout.row_width(), cols))
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
    selected: bool,
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
    let mut style = theme
        .pager
        .kind
        .for_kind(cand.kind.as_deref().unwrap_or(""));
    // On the selected row every kind takes one colour. Keeping the usual per-kind background
    // there would put a second highlight inside the row that is already highlighted.
    if selected && theme.pager.kind_sel.is_some() {
        style.bg = theme.pager.kind_sel;
    }
    style.paint(&inner, depth)
}

#[cfg(test)]
mod tests {
    use super::super::width::display_width;
    use super::*;

    /// Build a candidate with a kind, which is what the badge column reads.
    fn kinded(display: &str, kind: &str, desc: Option<&str>) -> CompletionCandidate {
        CompletionCandidate {
            display: display.to_string(),
            replacement: display.to_string(),
            description: desc.map(str::to_string),
            kind: Some(kind.to_string()),
            path: None,
            detail: None,
        }
    }

    /// A file candidate whose size the info column can report.
    fn sized_file(name: &str, bytes: &[u8]) -> (tempfile::TempDir, CompletionCandidate) {
        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join(name);
        std::fs::write(&path, bytes).expect("write");
        let mut cand = kinded(name, "file", None);
        cand.path = Some(path.to_string_lossy().into_owned());
        (dir, cand)
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
        let rows: Vec<&str> = seen.lines().filter(|l| !l.trim().is_empty()).collect();
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
        let rows: Vec<&str> = seen.lines().filter(|l| !l.trim().is_empty()).collect();
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
        assert_eq!(lines, 6);
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
        // Three candidates, three rows: there is no border above or below them any more.
        assert_eq!(widths.len(), 3);
        assert!(
            widths.iter().all(|w| *w == widths[0]),
            "rows disagree: {widths:?}"
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
        assert_eq!(lines, 3);
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
            assert_eq!(lines, 2, "at {cols} columns");
        }
    }

    #[test]
    fn wrapped_rows_are_counted_as_the_physical_rows_they_occupy() {
        // Below the width any box fits in, the rows do wrap — and the count must say so, or the
        // cursor-up walks too few rows and paints over the prompt.
        // Four columns: narrower than the chrome alone, so every row must wrap. Six used to be
        // enough to force this and no longer is — the border went, and with it four cells of
        // overhead per row.
        let candidates = vec![cand("cargo", None); 2];
        let (rendered, lines) = render_vertical_dropdown_at_width(&candidates, 0, 8, 0, 4, "");
        let widths = row_widths(&rendered);
        let expected: usize = widths.iter().map(|w| physical_rows(*w, 4)).sum();
        assert_eq!(lines, expected);
        assert!(lines > widths.len(), "wrapping was not accounted for");
    }

    /// There is no border and no caption: the menu is marked out by its background alone.
    #[test]
    fn the_menu_has_no_border_or_caption() {
        let candidates: Vec<_> = (0..20).map(|i| cand(&format!("cmd{i}"), None)).collect();
        let (rendered, lines) = render_vertical_dropdown_at_width(&candidates, 9, 8, 0, 80, "");
        for drawn in ['│', '╭', '╮', '╰', '╯', '─'] {
            assert!(!rendered.contains(drawn), "{drawn} survived: {rendered:?}");
        }
        assert!(!rendered.contains("Suggestions"), "{rendered:?}");
        assert!(!rendered.contains("Tab/Enter"), "{rendered:?}");
        // Only the rows themselves, with no chrome above or below them.
        assert_eq!(lines, 8);
        // Selection 9 is on the second page: cmd8..=cmd15.
        assert!(rendered.contains("cmd9"));
        assert!(!rendered.contains("cmd0 "));
    }

    /// On the selected row every badge takes one colour, so there is not a second highlight
    /// inside the row that is already highlighted.
    #[test]
    fn the_selected_rows_badge_takes_its_own_colour() {
        let _held = crate::theme::held_at(crate::theme::Depth::Ansi256);
        let candidates = vec![kinded("a", "dir", None), kinded("b", "dir", None)];
        let (rendered, _) = render_vertical_dropdown_at_width(&candidates, 0, 8, 0, 80, "");
        let theme = crate::theme::current();
        let selected = theme.pager.kind_sel.expect("a selected-badge colour");
        let ordinary = theme.pager.kind.dir.bg.expect("a dir colour");
        assert_ne!(
            selected, ordinary,
            "the two must differ or the test proves nothing"
        );

        // Both kinds are `dir`, so the only difference between the rows is the selection.
        let rows: Vec<&str> = rendered.lines().filter(|l| l.contains("dir")).collect();
        assert_eq!(rows.len(), 2, "{rows:?}");
        assert!(
            rows[0].contains("48;5;242"),
            "selected badge: {:?}",
            rows[0]
        );
        assert!(
            rows[1].contains("48;5;240"),
            "unselected badge: {:?}",
            rows[1]
        );
    }

    /// Every unselected row carries the background, which is now the only thing that says where
    /// the menu is.
    #[test]
    fn every_row_is_drawn_on_the_background() {
        let _held = crate::theme::held_at(crate::theme::Depth::Ansi256);
        let candidates = vec![cand("one", None), cand("two", None)];
        let (rendered, _) = render_vertical_dropdown_at_width(&candidates, 0, 8, 0, 80, "");
        assert!(rendered.contains("48;5;236"), "no background: {rendered:?}");
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
        assert_eq!(lines, 1);
        assert!(rendered.contains("cargo"));
    }

    /// The third column: a file says how big it is, and it is drawn without the description
    /// column having anything in it.
    #[test]
    fn a_file_row_carries_its_size() {
        let (_dir, cand) = sized_file("Cargo.toml", &[b'x'; 4300]);
        let (out, _) = render_vertical_dropdown_at_width(&[cand], 0, 5, 0, 120, "");
        assert!(plain(&out).contains("4.2K"), "{:?}", plain(&out));
    }

    /// An alias says what it expands to, which is the one thing its name does not tell you.
    #[test]
    fn an_alias_row_carries_its_expansion() {
        let mut cand = kinded("gst", "alias", None);
        cand.detail = Some("git status --short".to_string());
        let (out, _) = render_vertical_dropdown_at_width(&[cand], 0, 5, 0, 120, "");
        let plain = plain(&out);
        assert!(plain.contains("git status --short"), "{plain:?}");
        assert!(
            plain.contains("alias"),
            "the badge still names the kind: {plain:?}"
        );
    }

    /// Whatever the columns say, the row still fits. This is the invariant the whole layout
    /// exists for, and an extra column is a new way to break it.
    #[test]
    fn extra_columns_never_widen_a_row_past_the_screen() {
        let (_dir, file) = sized_file("a-rather-long-file-name.toml", &[b'x'; 4300]);
        let mut alias = kinded("gst", "alias", Some("a description of some length"));
        alias.detail = Some("git status --short --branch --untracked-files=all".to_string());
        let candidates = vec![file, alias];
        for cols in [12usize, 20, 40, 60, 80, 120, 200] {
            let (out, _) = render_vertical_dropdown_at_width(&candidates, 0, 5, 8, cols, "");
            for row in plain(&out).lines().filter(|l| !l.is_empty()) {
                assert!(
                    display_width(row) <= cols,
                    "{cols} columns, row of {}: {row:?}",
                    display_width(row)
                );
            }
        }
    }
}

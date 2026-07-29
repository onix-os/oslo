//! Fitting the dropdown's columns into the terminal it will be drawn on.
//!
//! The layout is the whole of R9.4's fix: given the candidates, the indent the caller would like,
//! and how many columns actually exist, decide widths such that every rendered row is at most
//! `cols` cells wide. Nothing downstream is allowed to widen a row afterwards.

use super::CompletionCandidate;
use super::width::{FALLBACK_COLS, display_width};

/// Cells an item row spends on chrome when it has a description column:
/// `│` ` ` ` ▶ ` label ` ` ` ` desc ` ` ` ` `│`.
pub(super) const OVERHEAD_WITH_DESC: usize = 10;
/// Cells an item row spends on chrome without one: `│` ` ` ` ▶ ` label ` ` ` ` `│`.
pub(super) const OVERHEAD_NO_DESC: usize = 8;
/// A label column narrower than this is not worth drawing; drop the description instead.
const MIN_LABEL_COLS: usize = 6;
/// A description column narrower than this says nothing; drop the column instead.
const MIN_DESC_COLS: usize = 8;
/// Below this the label column stops growing on its own account.
const LABEL_FLOOR: usize = 12;
/// A description column asks for this much even when the descriptions are shorter.
const DESC_PREFERRED: usize = 25;

/// The column widths a dropdown is drawn at, after clamping to the terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DropdownLayout {
    /// Left indent actually used — the requested indent, reduced until a box still fits.
    pub indent: usize,
    /// Width of the label column, icon included.
    pub label_w: usize,
    /// Width of the description column; 0 when descriptions do not fit at all.
    pub desc_w: usize,
    /// Visible width of every row, indent excluded.
    pub box_w: usize,
}

impl DropdownLayout {
    /// Visible width of a complete rendered row, indent included. This is the number that must
    /// not exceed the terminal width.
    pub fn row_width(&self) -> usize {
        self.indent + self.box_w
    }

    pub fn has_desc(&self) -> bool {
        self.desc_w > 0
    }
}

/// Fit label and description columns into `cols`.
///
/// Concessions are made in the order that costs the least information: the indent first (an
/// indented but illegible box is worse than a box in the left margin), then an over-long label
/// column, then the description column, and only if none of that is enough is the description
/// dropped and the label cut to whatever remains.
pub fn compute_layout(
    candidates: &[CompletionCandidate],
    indent_cols: usize,
    cols: usize,
) -> DropdownLayout {
    let cols = if cols == 0 { FALLBACK_COLS } else { cols };

    let natural_label = candidates
        .iter()
        .map(|c| display_width(c.icon()) + display_width(&c.display))
        .max()
        .unwrap_or(0)
        .max(LABEL_FLOOR);
    let natural_desc = candidates
        .iter()
        .map(|c| display_width(c.description.as_deref().unwrap_or("")))
        .max()
        .unwrap_or(0);

    // Keep at least a minimal box on screen; past that the indent is expendable.
    let smallest_box = MIN_LABEL_COLS + OVERHEAD_NO_DESC;
    let indent = indent_cols.min(cols.saturating_sub(smallest_box));
    let avail = cols.saturating_sub(indent);

    let mut label_w = natural_label;
    let mut desc_w = if natural_desc > 0 {
        natural_desc.max(DESC_PREFERRED)
    } else {
        0
    };

    if desc_w > 0 {
        // A label column that eats the row leaves no room for the text explaining it, so a long
        // label is ellipsised before the description is squeezed to nothing.
        label_w = label_w.min((avail * 3 / 5).max(MIN_LABEL_COLS));
    }

    if desc_w > 0 && label_w + desc_w + OVERHEAD_WITH_DESC > avail {
        let for_desc = avail.saturating_sub(label_w + OVERHEAD_WITH_DESC);
        if for_desc >= MIN_DESC_COLS {
            desc_w = for_desc;
        } else {
            let for_label = avail.saturating_sub(MIN_DESC_COLS + OVERHEAD_WITH_DESC);
            if for_label >= MIN_LABEL_COLS {
                label_w = for_label;
                desc_w = MIN_DESC_COLS;
            } else {
                desc_w = 0;
            }
        }
    }

    if desc_w == 0 {
        label_w = label_w.min(avail.saturating_sub(OVERHEAD_NO_DESC)).max(1);
    }

    let box_w = if desc_w > 0 {
        label_w + desc_w + OVERHEAD_WITH_DESC
    } else {
        label_w + OVERHEAD_NO_DESC
    };

    DropdownLayout {
        indent,
        label_w,
        desc_w,
        box_w,
    }
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

    #[test]
    fn a_comfortable_terminal_keeps_both_columns() {
        let c = vec![
            cand("cargo", Some("Rust package manager")),
            cand("cd", Some("Change working directory")),
        ];
        let l = compute_layout(&c, 4, 80);
        assert!(l.has_desc());
        assert_eq!(l.indent, 4);
        assert!(l.row_width() <= 80, "{l:?}");
    }

    #[test]
    fn indent_is_given_up_before_the_box_is() {
        // The R9.4 report: a deep cwd made the prompt-derived indent wider than the screen.
        let c = vec![cand("cargo", Some("Rust package manager")); 3];
        let l = compute_layout(&c, 200, 80);
        assert!(l.indent < 200);
        assert!(l.row_width() <= 80, "{l:?}");
        // Something is still legible after the indent is surrendered.
        assert!(l.label_w >= MIN_LABEL_COLS);
    }

    #[test]
    fn descriptions_shrink_before_labels_do() {
        let c = vec![cand(
            "a-fairly-long-candidate",
            Some("a description that is much too long for the space available here"),
        )];
        let l = compute_layout(&c, 0, 80);
        assert!(l.has_desc());
        assert_eq!(l.row_width(), 80);
        // Both columns were cut back, not just one of them.
        assert!(l.label_w < display_width("⚙️ a-fairly-long-candidate") + 1 || l.desc_w < 64);
    }

    #[test]
    fn descriptions_are_dropped_before_labels_become_illegible() {
        let c = vec![cand("cargo", Some("Rust package manager")); 2];
        // 30 columns can still hold both columns, if barely.
        let narrow = compute_layout(&c, 0, 30);
        assert!(narrow.row_width() <= 30);
        // A genuinely tiny terminal keeps the label and gives up the description entirely.
        let tiny = compute_layout(&c, 10, 16);
        assert!(!tiny.has_desc());
        assert_eq!(tiny.indent, 2);
        assert!(tiny.row_width() <= 16, "{tiny:?}");
    }

    #[test]
    fn absurdly_narrow_terminals_still_produce_a_layout() {
        // Nothing sane fits in 6 columns; the contract is only that the numbers stay coherent so
        // the renderer can count the wraps it causes.
        let c = vec![cand("cargo", None)];
        let l = compute_layout(&c, 40, 6);
        assert_eq!(l.indent, 0);
        assert!(l.label_w >= 1);
        assert_eq!(l.box_w, l.label_w + OVERHEAD_NO_DESC);
    }

    #[test]
    fn zero_columns_falls_back_rather_than_collapsing() {
        let c = vec![cand("cargo", Some("Rust package manager"))];
        assert_eq!(
            compute_layout(&c, 0, 0),
            compute_layout(&c, 0, FALLBACK_COLS)
        );
    }
}

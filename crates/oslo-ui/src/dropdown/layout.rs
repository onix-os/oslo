//! Fitting the dropdown's columns into the terminal it will be drawn on.
//!
//! The layout is the whole of R9.4's fix: given the candidates, the indent the caller would like,
//! and how many columns actually exist, decide widths such that every rendered row is at most
//! `cols` cells wide. Nothing downstream is allowed to widen a row afterwards.
//!
//! A row is a label, an optional kind badge, and *any number* of info columns — the description
//! first, then whatever [`super::columns`] adds, then whatever a config asks for on top. Since the
//! count is not known ahead of time, the widths are not computed by a cascade of special cases but
//! by making concessions in a fixed order until the row fits — see [`compute_layout`].

use super::width::{FALLBACK_COLS, display_width};

/// Cells an item row spends on chrome regardless of what is in it: ` ▶ ` label ` `.
/// There is no border; the row's background is what marks it out.
pub(super) const OVERHEAD_NO_DESC: usize = 4;
/// Cells between the label (or badge) and an info column, and between two info columns.
const GUTTER: usize = 2;
/// A label column narrower than this is not worth drawing; drop a column instead.
const MIN_LABEL_COLS: usize = 6;

/// An info column narrower than this says nothing; drop the column instead.
const MIN_COL_COLS: usize = 8;
/// Below this the label column stops growing on its own account.
///
/// It is also how much label the *indent* will be surrendered for: between this and
/// [`MIN_LABEL_COLS`] is the difference between a menu that tells its candidates apart and one that
/// shows four identical stubs.
const LABEL_FLOOR: usize = 12;
/// A badge narrower than this cannot spell the shortest kind (` dir `), so it is dropped whole.
const MIN_BADGE_COLS: usize = 5;

/// The column widths a dropdown is drawn at, after clamping to the terminal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DropdownLayout {
    /// Left indent actually used — the requested indent, reduced until a box still fits.
    pub indent: usize,
    /// Width of the label column.
    pub label_w: usize,
    /// Width of the kind badge column; 0 when badges do not fit or no candidate has a kind.
    pub badge_w: usize,
    /// Width of each info column that survived, left to right. Empty when none did.
    pub columns: Vec<usize>,
    /// Visible width of every row, indent excluded.
    pub box_w: usize,
}

impl DropdownLayout {
    /// Visible width of a complete rendered row, indent included. This is the number that must
    /// not exceed the terminal width.
    pub fn row_width(&self) -> usize {
        self.indent + self.box_w
    }

    /// Whether the description column — the first info column — is drawn.
    pub fn has_desc(&self) -> bool {
        !self.columns.is_empty()
    }

    pub fn has_badge(&self) -> bool {
        self.badge_w > 0
    }

    /// Chrome cells for this layout: the fixed frame, the badge and its space, and a gutter
    /// before each info column.
    fn overhead(&self) -> usize {
        OVERHEAD_NO_DESC
            + if self.badge_w > 0 {
                self.badge_w + 1
            } else {
                0
            }
            + self.columns.len() * GUTTER
    }

    fn width(&self) -> usize {
        self.label_w + self.columns.iter().sum::<usize>() + self.overhead()
    }
}

/// Fit the label, badge and info columns into `cols`.
///
/// `cells[i]` is the info-column text for candidate `i`, already squared off by
/// the `columns` module so every row has the same count.
///
/// Width is given up in a fixed order. This list is the only thing about the module worth
/// remembering, because "which column vanished first" is the question every report about a narrow
/// terminal turns out to be:
///
/// 1. **Indent.** An indented illegible box is worse than a box in the left margin.
/// 2. **Extra info columns, rightmost first.** A config that asks for five columns on an
///    eighty-column terminal gets the ones it named first; the rest are dropped whole rather than
///    all of them squeezed to three cells each.
/// 3. **The first info column, squeezed** to `MIN_COL_COLS`. Ahead of the badge because a text
///    column gives up its tail and stays readable, while a badge either fits or is gone.
/// 4. **The badge.**
/// 5. **The first info column, squeezed further** now the badge's cells are free — with the label
///    giving up the difference where that keeps both alive.
/// 6. **The first info column, dropped.**
/// 7. **The label, truncated.** Never dropped: a row with no label is not a completion.
pub fn compute_layout(
    candidates: &[super::CompletionCandidate],
    cells: &[Vec<String>],
    indent_cols: usize,
    cols: usize,
) -> DropdownLayout {
    let cols = if cols == 0 { FALLBACK_COLS } else { cols };

    let natural_label = candidates
        .iter()
        .map(|c| display_width(&c.display))
        .max()
        .unwrap_or(0)
        .max(LABEL_FLOOR);
    // Every badge in a dropdown is the same width, so the pills line up into a column rather than
    // ragging along the labels. The widest kind present sets it.
    let natural_badge = candidates
        .iter()
        .filter_map(|c| c.badge())
        .map(|b| display_width(b) + 2)
        .max()
        .unwrap_or(0);

    // Each column asks for exactly what its widest cell needs, and no more. A column used to be
    // able to ask for a comfortable *preferred* width beyond its contents; that only ever added
    // trailing blanks, and on a narrow terminal it bought them by dropping the columns to its
    // right — so a listing of short descriptions paid for its own padding with the size column.
    let column_count = cells.iter().map(Vec::len).max().unwrap_or(0);
    let mut columns: Vec<usize> = (0..column_count)
        .map(|i| {
            cells
                .iter()
                .map(|row| display_width(row.get(i).map(String::as_str).unwrap_or("")))
                .max()
                .unwrap_or(0)
        })
        .collect();
    // A column nothing fills is a gutter with a name. Drop it before it costs anything.
    while columns.last() == Some(&0) {
        columns.pop();
    }

    // Keep a *readable* box on screen; past that the indent is expendable. (Concession 1.)
    //
    // **The floor is what the labels actually need, not the bare minimum.** `MIN_LABEL_COLS` is 6,
    // and surrendering the indent only down to that meant a word typed near the right edge got six
    // cells to show every candidate in — so `alpha`, `alberta`, `albacore` and `album` all rendered
    // as the same stub and the menu said nothing at all. The indent is decoration; the labels are
    // the point, so the labels are asked first and the indent gives way to them.
    let readable = natural_label.clamp(MIN_LABEL_COLS, LABEL_FLOOR);
    let smallest_box = readable + OVERHEAD_NO_DESC;
    let indent = indent_cols.min(cols.saturating_sub(smallest_box));
    let avail = cols.saturating_sub(indent);

    let mut layout = DropdownLayout {
        indent,
        label_w: natural_label,
        badge_w: if natural_badge >= MIN_BADGE_COLS {
            natural_badge
        } else {
            0
        },
        columns,
        box_w: 0,
    };

    // A label column that eats the row leaves no room for the text explaining it, so a long label
    // is ellipsised before any column is squeezed to nothing. This is a cap, not a concession: it
    // applies whether or not the row already fits.
    if layout.has_desc() {
        layout.label_w = layout.label_w.min((avail * 3 / 5).max(MIN_LABEL_COLS));
    }

    // Concession 2: extra columns, rightmost first.
    while layout.width() > avail && layout.columns.len() > 1 {
        layout.columns.pop();
    }
    // Concession 3: squeeze the first column, down to `MIN_COL_COLS` but no further.
    //
    // Ahead of the badge, and that ordering was decided by looking at a real listing: two aliases
    // whose expansions ran to eighty cells took the whole row, and the badge column — nine cells
    // saying `alias` — was dropped to pay for the tail of an expansion that was going to be
    // ellipsised anyway. A text column gives up its end and stays readable; a badge is fixed and
    // either fits or is gone, so its last cells are worth more than a description's.
    if layout.width() > avail
        && let Some(&first) = layout.columns.first()
    {
        let room = avail.saturating_sub(layout.width() - first);
        layout.columns[0] = first.min(room.max(MIN_COL_COLS));
    }
    // Concession 4: the badge.
    if layout.width() > avail {
        layout.badge_w = 0;
    }
    // Concessions 5 and 6: squeeze again now the badge has gone, and failing that give the column
    // up — letting the label pay the difference where that keeps both alive.
    if layout.width() > avail
        && let Some(&first) = layout.columns.first()
    {
        let room = avail.saturating_sub(layout.width() - first);
        if room >= MIN_COL_COLS {
            layout.columns[0] = first.min(room);
        } else {
            // Everything the row spends that is neither the label nor this column.
            let fixed = layout.width() - first - layout.label_w;
            if avail.saturating_sub(fixed) >= MIN_COL_COLS + MIN_LABEL_COLS {
                layout.columns[0] = MIN_COL_COLS;
                layout.label_w = avail - fixed - MIN_COL_COLS;
            } else {
                layout.columns.clear();
            }
        }
    }
    // Concession 7: the label, truncated but never dropped.
    if layout.width() > avail {
        layout.label_w = avail.saturating_sub(layout.width() - layout.label_w).max(1);
    }

    layout.box_w = layout.width();
    layout
}

#[cfg(test)]
mod tests {
    use super::super::CompletionCandidate;
    use super::*;

    fn cand(display: &str, desc: Option<&str>) -> CompletionCandidate {
        CompletionCandidate::new(
            display.to_string(),
            display.to_string(),
            desc.map(str::to_string),
        )
    }

    /// Lay out the candidates with their descriptions as the only info column.
    fn layout(c: &[CompletionCandidate], indent: usize, cols: usize) -> DropdownLayout {
        let cells: Vec<Vec<String>> = c
            .iter()
            .map(|c| vec![c.description.clone().unwrap_or_default()])
            .collect();
        compute_layout(c, &cells, indent, cols)
    }

    fn with_columns(
        c: &[CompletionCandidate],
        extra: &[&str],
        indent: usize,
        cols: usize,
    ) -> DropdownLayout {
        let cells: Vec<Vec<String>> = c
            .iter()
            .map(|c| {
                let mut row = vec![c.description.clone().unwrap_or_default()];
                row.extend(extra.iter().map(|s| s.to_string()));
                row
            })
            .collect();
        compute_layout(c, &cells, indent, cols)
    }

    #[test]
    fn a_comfortable_terminal_keeps_both_columns() {
        let c = vec![
            cand("cargo", Some("Rust package manager")),
            cand("cd", Some("Change working directory")),
        ];
        let l = layout(&c, 4, 80);
        assert!(l.has_desc());
        assert_eq!(l.indent, 4);
        assert!(l.row_width() <= 80, "{l:?}");
    }

    #[test]
    fn indent_is_given_up_before_the_box_is() {
        // The R9.4 report: a deep cwd made the prompt-derived indent wider than the screen.
        let c = vec![cand("cargo", Some("Rust package manager")); 3];
        let l = layout(&c, 200, 80);
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
        let l = layout(&c, 0, 80);
        assert!(l.has_desc());
        assert!(l.row_width() <= 80, "{l:?}");
        // Both columns were cut back, not just one of them.
        assert!(l.label_w < display_width("a-fairly-long-candidate") + 1 || l.columns[0] < 64);
    }

    #[test]
    fn descriptions_are_dropped_before_labels_become_illegible() {
        let c = vec![cand("cargo", Some("Rust package manager")); 2];
        // 30 columns can still hold both columns, if barely.
        let narrow = layout(&c, 0, 30);
        assert!(narrow.row_width() <= 30, "{narrow:?}");
        // A genuinely tiny terminal keeps the label and gives up the description entirely.
        let tiny = layout(&c, 10, 16);
        assert!(!tiny.has_desc(), "{tiny:?}");
        assert!(tiny.row_width() <= 16, "{tiny:?}");
    }

    /// **The indent gives way to the labels, not the other way round.**
    ///
    /// A word typed near the right edge leaves a large indent, and the indent was only surrendered
    /// down to `MIN_LABEL_COLS` — six cells. Four candidates sharing a prefix then rendered as four
    /// identical stubs, which is a menu that has stopped saying anything. The indent is decoration;
    /// the labels are the whole point.
    #[test]
    fn a_late_word_does_not_crush_the_labels_to_a_stub() {
        let c = vec![
            cand("albacore", None),
            cand("alberta", None),
            cand("album", None),
            cand("alpha", None),
        ];
        // Typed 60 columns into a 70-column terminal: the natural indent cannot be afforded.
        let l = layout(&c, 60, 70);
        assert!(
            l.label_w >= LABEL_FLOOR,
            "the labels were crushed to {} cells: {l:?}",
            l.label_w
        );
        assert!(l.row_width() <= 70, "{l:?}");
    }

    #[test]
    fn absurdly_narrow_terminals_still_produce_a_layout() {
        // Nothing sane fits in 6 columns; the contract is only that the numbers stay coherent so
        // the renderer can count the wraps it causes.
        let c = vec![cand("cargo", None)];
        let l = layout(&c, 40, 6);
        assert_eq!(l.indent, 0);
        assert!(l.label_w >= 1);
        assert_eq!(l.box_w, l.label_w + OVERHEAD_NO_DESC);
    }

    #[test]
    fn zero_columns_falls_back_rather_than_collapsing() {
        let c = vec![cand("cargo", Some("Rust package manager"))];
        assert_eq!(layout(&c, 0, 0), layout(&c, 0, FALLBACK_COLS));
    }

    /// The new part: a row can carry as many columns as the config asks for, and a wide terminal
    /// simply draws them.
    #[test]
    fn a_wide_terminal_draws_every_column_it_was_given() {
        let c = vec![cand("Cargo.toml", Some("manifest"))];
        let l = with_columns(&c, &["4.2K", "2d", "rw-r--r--"], 0, 160);
        assert_eq!(l.columns.len(), 4);
        assert_eq!(l.columns[1], display_width("4.2K"));
        assert_eq!(l.columns[3], display_width("rw-r--r--"));
        assert!(l.row_width() <= 160, "{l:?}");
    }

    /// Concession 2: the columns named last are the ones that go, whole, rather than every column
    /// being squeezed into uselessness together.
    ///
    /// Asserted as an ordering over widths rather than against a magic column count, because the
    /// exact width at which a column goes is a function of the contents and moves whenever the
    /// formatting does — while the *order* is the promise.
    #[test]
    fn narrow_terminals_drop_extra_columns_from_the_right() {
        let c = vec![cand("Cargo.toml", Some("the package manifest"))];
        let extra = ["4.2K", "2d", "rw-r--r--"];

        let mut previous = usize::MAX;
        for cols in (10..=160usize).rev() {
            let l = with_columns(&c, &extra, 0, cols);
            // Narrowing never brings a column back.
            assert!(l.columns.len() <= previous, "{cols} columns: {l:?}");
            previous = l.columns.len();
            // And the row fits, at every width, all the way down.
            assert!(
                l.row_width() <= cols.max(MIN_LABEL_COLS + OVERHEAD_NO_DESC),
                "{cols} columns: {l:?}"
            );
        }
        // A wide terminal draws all four; a hopeless one draws none.
        assert_eq!(with_columns(&c, &extra, 0, 160).columns.len(), 4);
        assert_eq!(with_columns(&c, &extra, 0, 10).columns.len(), 0);
    }

    /// Concession 3: the badge goes *after* the extra columns, never before them. So a layout
    /// that still has more than the description showing must still have its badge.
    #[test]
    fn the_badge_outlives_the_columns_it_describes() {
        let mut c = cand("Cargo.toml", Some("the package manifest"));
        c.kind = Some("file".to_string());
        let c = vec![c];
        let extra = ["4.2K", "2d"];

        let mut saw_badge_dropped = false;
        for cols in (10..=160usize).rev() {
            let l = with_columns(&c, &extra, 0, cols);
            if l.columns.len() > 1 {
                assert!(l.has_badge(), "{cols} columns: {l:?}");
                assert!(
                    !saw_badge_dropped,
                    "{cols} columns: the badge came back: {l:?}"
                );
            }
            saw_badge_dropped |= !l.has_badge();
        }
        assert!(saw_badge_dropped, "the badge must go at some width");
    }

    /// A column no row fills never reaches the layout, but if one does it costs nothing.
    #[test]
    fn empty_trailing_columns_are_not_given_width() {
        let c = vec![cand("cargo", Some("Rust package manager"))];
        let l = with_columns(&c, &["", ""], 0, 120);
        assert_eq!(l.columns.len(), 1, "{l:?}");
    }
}

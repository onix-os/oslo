//! How the list inside a widget is drawn: where the filter sits, what is beside it, and what
//! colour everything takes.
//!
//! [`super::chrome::Chrome`] is what surrounds a widget — the border, the legend, the screen. This
//! is the other half: the rows themselves. The split is the useful one because the two are
//! genuinely independent. A bordered inline `choose` and a full-screen history browser differ in
//! `Chrome` *and* in `Look`, and either can be changed without touching the other.
//!
//! # Everything here is a default, not a rule
//!
//! The history finder is not a different program from `ui filter`; it is the same list with the
//! filter at the bottom, the rows growing up towards it, a tinted surface under the query and a
//! quiet stripe on every other row. All of those are fields, so a script can ask for them — which
//! is what [`Preset::History`] does in one word:
//!
//! ```sh
//! history | ui filter --look history --fullscreen
//! ```
//!
//! # The filter row has slots
//!
//! `left` and `right` are templates drawn either side of the query, which is where a list puts the
//! facts *about* itself: how many rows matched, which profile is being searched, where you are in
//! the list. Without them every widget that wanted a counter had to grow its own flag.

use crate::scanner::Scanner;
use crate::term::Key;
use crate::theme::{self, Color, Style};

mod bar;
mod paint;
#[cfg(test)]
#[path = "look/tests.rs"]
mod tests;

pub use paint::{Row, View};

/// Which end of the widget the filter sits at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Where {
    /// Above the list. The list reads top-down from the query, which is what a short menu wants.
    #[default]
    Top,
    /// Below the list, where the cursor already is. With [`Look::reverse`] the best match sits
    /// against it — fzf and atuin both landed here for the same reason.
    Bottom,
}

impl Where {
    pub fn parse(text: &str) -> Option<Where> {
        match text.trim().to_ascii_lowercase().as_str() {
            "top" | "above" | "start" => Some(Where::Top),
            "bottom" | "below" | "end" => Some(Where::Bottom),
            _ => None,
        }
    }
}

/// How wide a row is painted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Width {
    /// Only as wide as its own text. A selected row's colour stops where the text does.
    #[default]
    Content,
    /// Every row padded to the full width available, so a background reaches both edges. This is
    /// what makes a stripe read as a ruler rather than as a highlighted word.
    Full,
}

impl Width {
    pub fn parse(text: &str) -> Option<Width> {
        match text.trim().to_ascii_lowercase().as_str() {
            "content" | "text" | "hug" => Some(Width::Content),
            "full" | "wide" | "row" => Some(Width::Full),
            _ => None,
        }
    }
}

/// A whole look under one name.
///
/// Not sugar: these are the combinations that have to agree with each other. A bottom filter
/// without `reverse` puts the best match furthest from the cursor, and a stripe without `Full`
/// paints a coloured word rather than a ruler. Naming the working combinations means a script gets
/// the shape right without having to know why.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    /// What every widget did before this module existed.
    Plain,
    /// The history finder: filter at the bottom on a tinted surface, list growing up towards it,
    /// zebra stripes, full-width rows.
    History,
    /// Rows on a surface with no stripes — a menu that reads as one block.
    Menu,
}

impl Preset {
    pub fn parse(text: &str) -> Option<Preset> {
        match text.trim().to_ascii_lowercase().as_str() {
            "plain" | "none" | "default" => Some(Preset::Plain),
            "history" | "finder" | "atuin" => Some(Preset::History),
            "menu" | "block" => Some(Preset::Menu),
            _ => None,
        }
    }

    /// The look this name stands for, built from the current theme so a preset still follows it.
    pub fn look(self) -> Look {
        let plain = Look::default();
        match self {
            Preset::Plain => plain,
            Preset::History => {
                let pager = &theme::current().pager;
                Look {
                    filter_at: Where::Bottom,
                    reverse: true,
                    surface: pager.bg,
                    surface_rows: 3,
                    // A quiet fixed grey under every other row. Not `pager.bg`: that colour is the
                    // input surface, and a list wearing it would merge into the thing it sits
                    // above.
                    stripe: Some(Style {
                        bg: Some(Color::Indexed(235)),
                        ..Style::default()
                    }),
                    selected: Style {
                        bg: pager.sel_bg,
                        ..pager.text_sel
                    },
                    row: pager.text,
                    width: Width::Full,
                    prompt: "  >>  ".to_string(),
                    // `{badge} || 12/840`, with the badge the only part carrying a background.
                    right: "{badge} || {n}/{total} ".to_string(),
                    // One cell wider than hexe's default. The bar has the room, and a longer track
                    // gives the sweep somewhere to travel — at eight the head turns round almost
                    // as soon as it leaves.
                    scanner: Some(Scanner {
                        width: 9,
                        ..Scanner::default()
                    }),
                    meta_style: pager.column(1, false),
                    gap: 1,
                    pad: 1,
                    ..plain
                }
            }
            Preset::Menu => {
                let pager = &theme::current().pager;
                Look {
                    surface: pager.bg,
                    // The rows take the surface too, which is the difference between this and
                    // `History`: there is no stripe, so the colour is the only thing saying where
                    // the block starts and stops. A menu with a tinted filter above untinted rows
                    // reads as two things rather than one.
                    row: Style {
                        bg: pager.bg,
                        ..pager.text
                    },
                    selected: Style {
                        bg: pager.sel_bg,
                        ..pager.text_sel
                    },
                    width: Width::Full,
                    pad: 1,
                    ..plain
                }
            }
        }
    }
}

/// How the list inside a widget is drawn.
#[derive(Debug, Clone)]
pub struct Look {
    /// Which end the filter row sits at.
    pub filter_at: Where,
    /// Draw the list from the far end, so the best match is the row nearest the filter.
    pub reverse: bool,
    /// Template drawn at the left of the filter row, after the prompt. See [`Look::fill`].
    pub left: String,
    /// Template drawn hard against the right of the filter row.
    pub right: String,
    /// What marks where typing starts.
    pub prompt: String,
    /// What the query row says while it is empty.
    pub placeholder: String,
    /// The colour under the whole filter row, edge to edge.
    pub surface: Option<Color>,
    /// How many rows the filter takes. Three is a panel — a blank row, the query, a blank row —
    /// and reads as somewhere to type; one is a line.
    pub surface_rows: usize,
    /// Blank rows between the list and the filter.
    pub gap: usize,
    /// Blank columns each side of a list row, inside whatever the row is painted on.
    pub pad: usize,
    /// Drawn against the selected row, and its width reserved on every other one.
    pub marker: String,
    /// An ordinary row.
    pub row: Style,
    /// The row the cursor is on.
    pub selected: Style,
    /// Every other row, for a list long enough to lose your place in.
    pub stripe: Option<Style>,
    /// The marker, the prompt, and a checked box.
    pub accent: Style,
    /// The characters the query matched. Only those, not the row around them: a fuzzy hit is
    /// otherwise a mystery.
    pub hit: Style,
    /// The counter and anything else in the slots.
    pub muted: Style,
    /// Whether a row's colour stops at its text or reaches the edge.
    pub width: Width,
    /// Columns of untouched terminal down each side of every row.
    ///
    /// **Because the right-hand one is not optional.** `Chrome` reserves a column at the right edge
    /// — a row exactly the terminal's width leaves the cursor in the auto-wrap pending state, and
    /// the `\r\n` after it then costs two rows instead of one. So a full-width block already has a
    /// gap on the right and none on the left, which reads as a block that failed to reach the edge
    /// rather than as one with a margin. This puts the same gap on the other side.
    pub margin: usize,
    /// A row of its own under the filter, templated like [`Look::left`] — `{n}`, `{total}`.
    ///
    /// **A row rather than a slot on the filter line**, which is the difference between a finder
    /// that reads as a search and one that reads as a list with a label on it. Empty draws nothing
    /// and costs no row, so nothing that does not ask for it changes.
    pub under: String,
    /// The narrowest the filter surface may be, whatever [`Width::Content`] measures.
    ///
    /// **Set to the legend's width by the widget that draws one.** Without it a panel sized to its
    /// own text shrank as the query got shorter, so the tinted box breathed in and out under a rule
    /// that stayed put — and an empty query left it narrower than the keys listed beneath it. A
    /// floor is the whole fix: it never reaches past what the box is already, and never retreats
    /// behind it.
    pub min_width: usize,
    /// An animated sweep at the head of the filter row, drawn where a spinner would go.
    ///
    /// It says the widget is live. That matters most where the list is doing work you cannot see —
    /// searching a large history — and it is the reason the finder's bar does not read as frozen
    /// while you think about what to type. Costs a redraw every [`Scanner::step_ms`], which is why
    /// it is off unless asked for.
    pub scanner: Option<Scanner>,
    /// A piece of a slot with its own colour, substituted wherever `{badge}` appears.
    ///
    /// **The one part of the bar with a background**, because it is the only part that is a *state
    /// you can change from here* rather than a fact about what you are looking at. That is the
    /// distinction the finder draws between `[global]` and `12/840`, and it is worth keeping: a
    /// second coloured pill beside it would make both read as decoration.
    pub badge: String,
    pub badge_style: Style,
    /// The fixed-width columns before a row's text: how long ago, how many times, how big.
    ///
    /// Right-aligned as a block, always, so they form columns down the screen even though the text
    /// beside them varies wildly in length — which is the whole reason to have them. The eye can
    /// then scan one column without reading the others.
    pub meta_style: Style,
    /// How many of a row's fields are metadata rather than text. Only `table` has fields to split.
    pub meta_columns: usize,
}

impl Default for Look {
    /// Exactly what every widget drew before this module existed.
    fn default() -> Self {
        let ui = theme::current().ui;
        Look {
            filter_at: Where::Top,
            reverse: false,
            left: String::new(),
            right: String::new(),
            prompt: "> ".to_string(),
            placeholder: "type to filter".to_string(),
            surface: None,
            surface_rows: 1,
            gap: 0,
            pad: 0,
            marker: "> ".to_string(),
            row: Style::default(),
            selected: ui.accent,
            stripe: None,
            accent: ui.accent,
            hit: Style {
                fg: Some(Color::Indexed(0)),
                bg: Some(Color::Indexed(1)),
                ..Style::default()
            },
            muted: ui.muted,
            width: Width::Content,
            min_width: 0,
            margin: 0,
            under: String::new(),
            scanner: None,
            badge: String::new(),
            // Foreground 0 on background 1: the terminal's own palette, so it belongs to whatever
            // scheme is in use, and inverted enough to read against a tinted surface.
            badge_style: Style {
                fg: Some(Color::Indexed(0)),
                bg: Some(Color::Indexed(1)),
                ..Style::default()
            },
            meta_style: ui.muted,
            meta_columns: 0,
        }
    }
}

impl Look {
    /// Whether this draws what the widgets drew before `Look` existed.
    ///
    /// Worth a method rather than a comparison: it is what lets a widget keep its old code path
    /// untouched, and therefore what makes this module additive rather than a rewrite of six
    /// widgets at once.
    pub fn is_plain(&self) -> bool {
        let plain = Look::default();
        self.filter_at == plain.filter_at
            && !self.reverse
            && self.left.is_empty()
            && self.right.is_empty()
            && self.surface.is_none()
            && self.stripe.is_none()
            && self.gap == 0
            && self.pad == 0
            && self.width == plain.width
            && self.surface_rows == 1
            && self.prompt == plain.prompt
    }

    /// A slot template with its fields filled in.
    ///
    /// The names are what a list knows about itself and nothing more — no expressions, no
    /// conditionals. A template language here would be a second thing to learn for the sake of a
    /// counter; anything richer belongs in the caller, which can build the string itself.
    ///
    /// * `{n}` — rows matching the query
    /// * `{total}` — rows in the list
    /// * `{index}` — where the cursor is, counting from one
    /// * `{query}` — what has been typed
    /// * `{marked}` — how many rows are checked
    pub fn fill(template: &str, view: &View<'_>) -> String {
        if !template.contains('{') {
            return template.to_string();
        }
        template
            .replace("{n}", &view.matched.to_string())
            .replace("{total}", &view.total.to_string())
            .replace("{index}", &(view.selected + 1).to_string())
            .replace("{query}", view.query)
            .replace("{marked}", &view.marked.to_string())
    }

    /// How long a widget may wait for a key before it has to redraw, in milliseconds.
    ///
    /// `None` means nothing moves on its own and the widget can block until something is typed —
    /// which is what every one of them did before the scanner existed, and what they should go
    /// back to doing the moment it is off. An animation that costs a wakeup on an idle prompt is
    /// worth having only while it is being looked at.
    pub fn tick_ms(&self) -> Option<i32> {
        self.scanner.map(|s| s.step_ms.max(1) as i32)
    }

    /// Rows this look adds around the list, so a widget can reserve them before drawing.
    pub fn extra_rows(&self, filtering: bool) -> usize {
        match filtering {
            true => self.surface_rows + self.gap + usize::from(!self.under.is_empty()),
            false => 0,
        }
    }

    /// Where a movement key should take the cursor.
    ///
    /// **The arrows follow the screen, not the list.** A reversed list is drawn from the far end,
    /// so index 0 is the *bottom* row — and moving to index 1 walks the cursor upward. Left to the
    /// widgets, Up moved the highlight down and Down moved it up, which is not a preference to be
    /// argued about: the key is named for a direction and the cursor went the other way.
    ///
    /// Decided here rather than in each widget so `choose`, `table` and `file` cannot disagree
    /// about it — they did, once, and only one of them was right.
    pub fn step(&self, key: Key) -> Option<Step> {
        let step = match key {
            Key::Up => Step::Back,
            Key::Down => Step::On,
            Key::PageUp | Key::Home => Step::First,
            Key::PageDown | Key::End => Step::Last,
            _ => return None,
        };
        Some(match self.reverse {
            true => step.flipped(),
            false => step,
        })
    }
}

/// What a movement key does to the cursor, once the list's own direction is accounted for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Towards index zero.
    Back,
    /// Away from index zero.
    On,
    First,
    Last,
}

impl Step {
    fn flipped(self) -> Step {
        match self {
            Step::Back => Step::On,
            Step::On => Step::Back,
            Step::First => Step::Last,
            Step::Last => Step::First,
        }
    }

    /// `selected` after this step, over a list of `len` rows.
    pub fn from(self, selected: usize, len: usize) -> usize {
        let last = len.saturating_sub(1);
        match self {
            Step::Back => selected.saturating_sub(1),
            Step::On => (selected + 1).min(last),
            Step::First => 0,
            Step::Last => last,
        }
    }
}

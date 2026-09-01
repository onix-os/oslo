//! `explore` — reading a table that does not fit on the screen.
//!
//! The drawn table is one frame of output: it is clamped to the terminal, a wide row loses its last
//! columns to an ellipsis, and a nested cell says `<3 items>` because a row that is two lines is not
//! a row. All three are the right trade for something that scrolls past between two commands, and
//! all three are the wrong one when the table *is* what you came to look at. This is the other
//! answer — the same rows, on the alternate screen, where the screen can move instead of the data
//! being cut to fit it.
//!
//! ```text
//! ps | where 'rss > 1e8' | explore
//! docker inspect x | from json | explore
//! ```
//!
//! # What it is not
//!
//! Not an editor, and not a picker. It answers nothing and changes nothing: `explore` ends the
//! pipeline, so there is no next stage for a chosen row to reach, and a viewer that quietly had a
//! return value would be a second thing to explain. `ui table` is the picker, and it is a different
//! widget for a different question.
//!
//! # Why the data is its own type
//!
//! [`Sheet`] and [`Cell`] are plain strings and boxes rather than the shell's `Val`, because this
//! crate is underneath the shell and cannot see that type. The conversion is the shell's job, which
//! also means the *summary* on a nested cell — `<3 items>` — is written once, by the renderer that
//! already draws it, instead of being invented a second time here.

mod render;
mod run;

pub use run::{Outcome, open};

/// One table: what to call it, its columns, and its rows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Sheet {
    /// What this level is called, shown in the breadcrumb — the verb at the top, a column name
    /// below it.
    pub title: String,
    pub columns: Vec<String>,
    /// One flag per column: draw it right-aligned, because it holds numbers.
    ///
    /// **Decided by the shell, not here.** What reads as a number is a question about `4.2G` and
    /// `2m30s` and a path that starts with a digit, and the drawn table already answers it — a
    /// viewer that answered it a second time would eventually answer it differently, and a column
    /// that changed alignment when you opened it would look like different data.
    pub numeric: Vec<bool>,
    pub rows: Vec<Vec<Cell>>,
}

/// One cell: text, or a table of its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cell {
    /// Already one line, drawn as it is.
    Flat(String),
    /// A list or a record. `summary` is what the column shows — the same `<3 items>` the drawn
    /// table shows — and `sheet` is what Enter opens.
    Nested { summary: String, sheet: Box<Sheet> },
}

impl Cell {
    /// What this cell looks like in a column, at either depth.
    pub fn text(&self) -> &str {
        match self {
            Cell::Flat(text) => text,
            Cell::Nested { summary, .. } => summary,
        }
    }

    /// The table under this cell, if there is one.
    pub fn sheet(&self) -> Option<&Sheet> {
        match self {
            Cell::Flat(_) => None,
            Cell::Nested { sheet, .. } => Some(sheet),
        }
    }
}

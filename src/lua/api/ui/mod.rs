//! `oslo.ui` — drawing and asking, for scripts and configs.
//!
//! Four subjects, and they are separate files because they fail in different ways:
//!
//! * [`text`] measures and shapes text in **cells**, which is what a terminal has and what a
//!   string does not;
//! * [`layout`] puts text where it fits — columns and tables;
//! * [`ask`] takes an answer back from the person;
//! * the terminal facts below are what all three are relative to.
//!
//! # What this deliberately is not
//!
//! There is no screen, no cursor addressing, no alternate buffer, no widget that owns the terminal
//! until it decides to give it back. A shell's UI has to compose with a *scrolling transcript* —
//! everything drawn here is lines of text that stay where they were printed, and that a pipe, a
//! log or `tee` receives intact. A full-screen library would be a second, incompatible way to use
//! the terminal living inside the one that already works.

mod ask;
mod layout;
mod text;

use super::util::{ok, put};
use crate::interactive::dropdown::width;
use crate::interactive::theme;
use crate::lua::eval::value::{Number, Table, Value};
use std::io::IsTerminal;

/// Build `oslo.ui`, minus the pieces other modules contribute (`style`, `prompt`).
pub fn install(ui: &mut Table) {
    terminal(ui);
    text::install(ui);
    layout::install(ui);
    ask::install(ui);
}

/// What the terminal is, so a script can decide what to draw before drawing it.
fn terminal(ui: &mut Table) {
    // oslo.ui.width() / oslo.ui.height() -> cells and rows
    put(ui, "width", |_, _| {
        ok(Value::Number(Number::Int(width::terminal_cols() as i64)))
    });
    put(ui, "height", |_, _| {
        ok(Value::Number(Number::Int(width::terminal_rows() as i64)))
    });

    // oslo.ui.is_tty() -> whether anyone is actually looking
    //
    // stdout, because that is where output goes. A script that redirects its output and keeps its
    // terminal is asking the wrong question if it asks this — it wants to know whether to colour
    // what it is writing, and what it is writing is going to the pipe.
    put(ui, "is_tty", |_, _| {
        ok(Value::Bool(std::io::stdout().is_terminal()))
    });

    // oslo.ui.colors() -> "none" | "ansi16" | "ansi256" | "truecolor"
    //
    // The theme's own answer, so a script paints with exactly the depth the shell paints with
    // rather than re-deriving it from `$TERM` and disagreeing.
    put(ui, "colors", |_, _| {
        ok(Value::str(match theme::depth() {
            theme::Depth::None => "none",
            theme::Depth::Ansi16 => "ansi16",
            theme::Depth::Ansi256 => "ansi256",
            theme::Depth::True => "truecolor",
        }))
    });
}

#[cfg(test)]
mod tests {
    use crate::interactive::dropdown::width;

    /// Neither dimension may be zero, whatever the environment does.
    ///
    /// Every layout in this module divides by the width. A zero would not produce a wonky column,
    /// it would produce a division by zero or an infinite loop, in a shell, on somebody's terminal.
    #[test]
    fn the_terminal_always_has_a_size() {
        assert!(width::terminal_cols() > 0);
        assert!(width::terminal_rows() > 0);
    }
}

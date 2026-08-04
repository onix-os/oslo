//! Asking the person at the terminal something, from a script.
//!
//! This is oslo's answer to [gum](https://github.com/charmbracelet/gum): the widgets a shell
//! script needs to be interactive, available to both languages the shell reads. In shell they are
//! the `ui` builtin; in Lua they are `oslo.ui`. Both call the same code, so a prompt looks the
//! same whichever language asked for it.
//!
//! ```sh
//! name=$(ui input --placeholder "your name")
//! ui confirm "delete everything?" && rm -rf "$target"
//! branch=$(git branch | ui filter --header "check out")
//! ```
//!
//! ```lua
//! local name = oslo.ui.input{placeholder = "your name"}
//! if oslo.ui.confirm("delete everything?") then ... end
//! ```
//!
//! # The result goes to stdout, everything else to stderr
//!
//! A widget is a question, and the answer is the script's data. `name=$(ui input)` has to capture
//! the name and nothing else, so the prompt, the list, the cursor and the key legend are all
//! written to stderr — the same reason `read -p` puts its prompt there. That single rule is what
//! makes these composable in a pipeline instead of merely usable at a prompt.
//!
//! # Cancelling is a status, not an answer
//!
//! Esc and Ctrl-C exit non-zero and print nothing. A script can therefore write
//! `x=$(ui input) || exit` and mean it, where a widget that returned an empty string on cancel
//! would make "cancelled" and "typed nothing" indistinguishable — and gum gets this right for the
//! same reason.
//!
//! # No terminal, no widget
//!
//! With stdin not a terminal every widget refuses rather than blocking on a pipe that will never
//! deliver a keypress. `--default` says what to answer in that case, which is what makes a script
//! using these still work under CI.

mod choose;
mod confirm;
mod file;
mod format;
mod input;
mod join;
mod log;
mod pager;
mod spin;
mod style;
mod table;
mod write;

pub use choose::{Choice, choose, filter};
pub use confirm::{Confirm, confirm};
pub use file::{Browse, Want, file};
pub use format::{As, format};
pub use input::{Input, input};
pub use join::{Align, horizontal, vertical};
pub use log::{Entry, Level, line};
pub use pager::{Pager, pager};
pub use spin::{Spin, spin};
pub use style::{Border, Styling, style};
pub use table::{Table, parse as parse_table, table};
pub use write::{Write, write};

/// What a widget answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer<T> {
    /// The person answered.
    Given(T),
    /// Esc or Ctrl-C. Non-zero status, no output.
    Cancelled,
    /// There was no terminal to ask on, and no default was supplied.
    NoTerminal,
}

impl<T> Answer<T> {
    /// The exit status a script should see. gum's convention, and the one that makes
    /// `x=$(ui input) || exit` correct.
    pub fn status(&self) -> i32 {
        match self {
            Answer::Given(_) => 0,
            Answer::Cancelled => 1,
            Answer::NoTerminal => 2,
        }
    }

    pub fn given(self) -> Option<T> {
        match self {
            Answer::Given(value) => Some(value),
            _ => None,
        }
    }
}

/// Write to stderr, where a prompt belongs. See the module note.
pub(crate) fn show(text: &str) {
    use std::io::Write;
    let mut err = std::io::stderr();
    let _ = err.write_all(text.as_bytes());
    let _ = err.flush();
}

/// The key legend along the bottom of a widget.
///
/// Always shown, and dim. A prompt whose keys you have to guess is one you leave by closing the
/// window; the two rows a legend costs are cheaper than that once.
pub(crate) fn legend(keys: &[(&str, &str)]) -> String {
    let ui = crate::interactive::theme::current().ui;
    let depth = crate::interactive::theme::depth();
    let parts: Vec<String> = keys
        .iter()
        .map(|(key, what)| {
            format!(
                "{} {}",
                ui.accent.paint(key, depth),
                ui.muted.paint(what, depth)
            )
        })
        .collect();
    parts.join(ui.muted.paint("  ", depth).as_str())
}

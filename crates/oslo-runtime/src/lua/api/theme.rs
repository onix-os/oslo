//! `oslo.theme.style` — painting a string the way the rest of the shell paints one.
//!
//! ```lua
//! oslo.theme.define("warn", "fg:yellow bold")
//! print(oslo.theme.style("careful", "warn"))
//! print(oslo.theme.style("or inline", "fg:cyan underline"))
//! ```
//!
//! `oslo.theme` is otherwise a table a config fills in with data, read once at startup. These are
//! the entries that are functions, and they are here rather than in `oslo.ui` because what they
//! answer to is the theme: the same name a prompt segment styles with, resolved the same way, and
//! obeying the same `NO_COLOR`.
//!
//! # Not the same as `oslo.ui.style`
//!
//! `oslo.ui.style(text, { fg = "green" })` paints from a table written at the call site. This one
//! takes hexe's string spelling — the one `oslo.theme` is already written in — and, before parsing
//! it, looks the name up in the **defined** styles. That is the half `oslo.ui.style` cannot do: a
//! config names `warn` once and every caller follows when it changes.
//!
//! Pure, so a completion provider or a registered builtin can call them.

use super::util::{failed, ok, opt_text, put, text};
use oslo_base::value::{Table, Value};
use oslo_ui::theme::{Depth, styles};

/// Build the function half of `oslo.theme`.
pub fn build() -> Table {
    let mut theme = Table::new();

    // oslo.theme.style(text, spec) -> text with the escapes around it
    //
    // A defined name wins over the inline spelling, so a config can redefine `warn` in one place
    // and every caller follows. Empty at `NO_COLOR`, which is why the caller never has to check.
    put(&mut theme, "style", |_, args| {
        let subject = text(&args, 1, "oslo.theme.style")?;
        let spec = text(&args, 2, "oslo.theme.style")?;
        let style = styles::lookup(&spec).unwrap_or_else(|| styles::parse(&spec));
        ok(Value::str(style.paint(&subject, depth_now())))
    });

    // oslo.theme.define(name, spec) -> true
    put(&mut theme, "define", |_, args| {
        let name = text(&args, 1, "oslo.theme.define")?;
        let spec = text(&args, 2, "oslo.theme.define")?;
        styles::define(&name, &spec);
        ok(Value::Bool(true))
    });

    // oslo.theme.depth([name]) -> what this terminal will be painted at, and optionally set it
    put(&mut theme, "depth", |_, args| {
        if let Some(name) = opt_text(&args, 1, "oslo.theme.depth")? {
            match Depth::named(&name) {
                Some(depth) => oslo_ui::theme::set_depth(depth),
                None => return failed("oslo.theme.depth", format!("'{name}' is not a depth")),
            }
        }
        ok(Value::str(name_of(depth_now())))
    });

    theme
}

/// What the shell is painting at right now.
fn depth_now() -> Depth {
    oslo_ui::theme::depth()
}

/// The spelling `Depth::named` accepts back.
fn name_of(depth: Depth) -> &'static str {
    match depth {
        Depth::None => "none",
        Depth::Ansi16 => "16",
        Depth::Ansi256 => "256",
        Depth::True => "truecolor",
    }
}

#[cfg(test)]
#[path = "theme/tests.rs"]
mod tests;

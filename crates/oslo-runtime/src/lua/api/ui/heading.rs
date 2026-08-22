//! `oslo.ui.title` / `oslo.ui.subtitle` — a heading, and the quieter line under it.
//!
//! ```lua
//! print(oslo.ui.title("oslo 0.4.3"))
//! print(oslo.ui.subtitle("the static release binary"))
//! print(oslo.ui.title("done", { rule = false, fg = "green" }))
//! ```
//!
//! # Why these are functions and not four lines of `oslo.ui.style`
//!
//! Because everybody writes the four lines differently. A recipe, a plugin and a `.env.lua` each
//! headed their output with a bold line and a rule of their own invention, at their own width, in
//! their own colour — so output from one shell looked like output from three programs. This is the
//! same decision `oslo.ui.grid` makes about columns: the shape is worth having once.
//!
//! # The rule is measured, not assumed
//!
//! Its width is the title's own, in **cells** — an emoji or a CJK character is two, and a rule cut
//! to `#text` would come up short under exactly the headings people put emoji in. It is capped at
//! the terminal so a long title in a narrow pane does not wrap the rule onto a second row, which
//! reads as a drawing mistake rather than as a long title.

use super::super::util::{ok, put, text};
use oslo_base::value::{Table, Value};
use oslo_ui::dropdown::width;
use oslo_ui::ink::ink;
use oslo_ui::theme::{Color, Style};

/// The character a rule is drawn with.
const RULE: &str = "─";

pub fn install(ui: &mut Table) {
    // oslo.ui.title(text, [{ fg, rule, width }]) -> the heading, as a string
    put(ui, "title", |_, args| {
        let text = text(&args, 1, "oslo.ui.title")?;
        let options = args.get(1);
        let mut style = Style {
            bold: true,
            ..Style::default()
        };
        if let Some(colour) = colour_of(options, "fg") {
            style.fg = Some(colour);
        }
        let mut out = ink(&text).styled(style).to_string();
        if ruled(options) {
            let cells = rule_width(&text, options);
            out.push('\n');
            out.push_str(&ink(RULE.repeat(cells)).dim().to_string());
        }
        ok(Value::str(out))
    });

    // oslo.ui.subtitle(text, [{ fg }]) -> the quieter line under a title
    //
    // Dim rather than a colour, so it stays subordinate on a light background and a dark one alike
    // — a grey chosen for one is unreadable on the other.
    put(ui, "subtitle", |_, args| {
        let text = text(&args, 1, "oslo.ui.subtitle")?;
        let inked = match colour_of(args.get(1), "fg") {
            Some(colour) => ink(&text).fg(colour),
            None => ink(&text).dim(),
        };
        ok(Value::str(inked.to_string()))
    });
}

/// A colour named in the options table, if there is one and it reads as a colour.
fn colour_of(options: Option<&Value>, key: &str) -> Option<Color> {
    let Some(Value::Table(t)) = options else {
        return None;
    };
    let named = t.borrow().get_str(key);
    match named {
        Value::Str(name) => Color::parse(&name),
        _ => None,
    }
}

/// Whether to draw the rule. Absent means yes; that is what a title is.
fn ruled(options: Option<&Value>) -> bool {
    let Some(Value::Table(t)) = options else {
        return true;
    };
    !matches!(t.borrow().get_str("rule"), Value::Bool(false))
}

/// How wide the rule is: what the caller asked for, else the title's own width in cells, and never
/// wider than the terminal.
fn rule_width(text: &str, options: Option<&Value>) -> usize {
    let asked = match options {
        Some(Value::Table(t)) => t
            .borrow()
            .get_str("width")
            .as_number()
            .and_then(|n| n.as_int())
            .map(|n| n.max(0) as usize),
        _ => None,
    };
    let wanted = asked.unwrap_or_else(|| width::display_width(text));
    wanted.min(width::terminal_cols()).max(1)
}

#[cfg(test)]
#[path = "heading/tests.rs"]
mod tests;

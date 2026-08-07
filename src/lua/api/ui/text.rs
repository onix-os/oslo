//! Measuring and shaping text for a terminal, which is not the same as measuring a string.
//!
//! Every function here counts **cells**, not bytes and not `char`s. Three things make those differ,
//! and a shell hits all three daily:
//!
//! * an escape sequence occupies no cells at all, so a coloured string is wider than it looks to
//!   `#s` and narrower than it looks to the terminal;
//! * a CJK character or an emoji occupies two;
//! * a combining mark or a zero-width joiner occupies none, and belongs to the character before it.
//!
//! Getting that wrong is not cosmetic. A column laid out on `#s` wraps, and a wrapped line in a
//! prompt or a dropdown corrupts the redraw that follows it. This is why the measuring lives in
//! Rust and is shared with the dropdown rather than being left for a config to approximate.

use super::super::util::{ok, put, text};
use crate::lua::eval::value::{Number, Table, Value};
use crate::ui::dropdown::width;

/// An optional non-negative integer argument.
fn count(args: &[Value], at: usize) -> Option<usize> {
    match args.get(at) {
        Some(Value::Number(n)) => {
            let n = n.as_float();
            (n >= 0.0).then_some(n as usize)
        }
        _ => None,
    }
}

pub fn install(ui: &mut Table) {
    // oslo.ui.width_of(s) -> cells
    put(ui, "width_of", |_, args| {
        let s = text(&args, 1, "oslo.ui.width_of")?;
        ok(Value::Number(Number::Int(width::display_width(&s) as i64)))
    });

    // oslo.ui.strip(s) -> s without escapes, i.e. what the terminal shows
    put(ui, "strip", |_, args| {
        let s = text(&args, 1, "oslo.ui.strip")?;
        ok(Value::str(width::without_escapes(&s)))
    });

    // oslo.ui.truncate(s, cells) -> s cut to fit, ending in `…` when something was dropped
    put(ui, "truncate", |_, args| {
        let s = text(&args, 1, "oslo.ui.truncate")?;
        let max = count(&args, 1).unwrap_or(0);
        ok(Value::str(width::truncate_to_width(&s, max)))
    });

    // oslo.ui.pad(s, cells, [align]) -> s padded to width; align is "left", "right" or "center"
    put(ui, "pad", |_, args| {
        let s = text(&args, 1, "oslo.ui.pad")?;
        let target = count(&args, 1).unwrap_or(0);
        let align = match args.get(2) {
            Some(Value::Str(a)) => a.to_string(),
            _ => "left".to_string(),
        };
        ok(Value::str(align_to(&s, target, &align)))
    });

    // oslo.ui.fit(s, cells) -> exactly this many cells: truncated if long, padded if short
    //
    // The one a column actually wants. Doing it as two calls is the common way to end up one cell
    // out, because `truncate` may return *less* than the budget when the cut lands mid-character.
    put(ui, "fit", |_, args| {
        let s = text(&args, 1, "oslo.ui.fit")?;
        let target = count(&args, 1).unwrap_or(0);
        let cut = width::truncate_to_width(&s, target);
        ok(Value::str(width::pad_to_width(&cut, target)))
    });
}

/// Pad `s` to `target` cells on the side `align` names.
///
/// An unknown alignment pads left rather than raising: this is called per cell of a table being
/// drawn, and a typo in a config should give you a wonky column, not a stack trace half way
/// through rendering.
pub fn align_to(s: &str, target: usize, align: &str) -> String {
    let have = width::display_width(s);
    if have >= target {
        return s.to_string();
    }
    let slack = target - have;
    match align {
        "right" => format!("{}{s}", " ".repeat(slack)),
        "center" | "centre" => {
            let left = slack / 2;
            format!("{}{s}{}", " ".repeat(left), " ".repeat(slack - left))
        }
        _ => format!("{s}{}", " ".repeat(slack)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The reason this is not `#s`: colour is free, and CJK is not.
    #[test]
    fn escapes_are_free_and_wide_characters_are_not() {
        assert_eq!(width::display_width("\x1b[31mred\x1b[0m"), 3);
        assert_eq!(width::display_width("日本"), 4);
    }

    #[test]
    fn stripping_leaves_exactly_what_the_terminal_shows() {
        assert_eq!(width::without_escapes("\x1b[1;31mred\x1b[0m"), "red");
        assert_eq!(width::without_escapes("\x1b]0;title\x07plain"), "plain");
        assert_eq!(
            width::without_escapes("nothing to strip"),
            "nothing to strip"
        );
    }

    /// Every alignment lands on the requested width, including for text that is already wide.
    #[test]
    fn padding_is_exact_in_cells_whichever_side_it_is_on() {
        for align in ["left", "right", "center"] {
            assert_eq!(
                width::display_width(&align_to("ab", 6, align)),
                6,
                "{align}"
            );
            // Colour must not make it pad short — the escapes are not cells.
            let painted = align_to("\x1b[31mab\x1b[0m", 6, align);
            assert_eq!(width::display_width(&painted), 6, "{align} with colour");
        }
        assert_eq!(align_to("already wide", 3, "left"), "already wide");
    }

    #[test]
    fn centring_puts_the_odd_cell_on_the_right() {
        assert_eq!(align_to("ab", 5, "center"), " ab  ");
    }

    /// An alignment nobody defined must not stop a table half-drawn.
    #[test]
    fn an_unknown_alignment_pads_rather_than_raising() {
        assert_eq!(align_to("ab", 4, "sideways"), "ab  ");
    }
}

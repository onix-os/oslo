//! `oslo.segment` — a prompt built from named pieces rather than one string.
//!
//! A prompt function can already return anything, so why a segment at all: because a *string* is
//! opaque. Once the prompt is one string, oslo cannot tell the user's name from the branch, cannot
//! drop the least important piece when the terminal is narrow, and cannot restyle one part without
//! the config rewriting all of it. A list of named pieces can be measured, reordered, styled and
//! *dropped* individually.
//!
//! The shape is deliberately hexe's, so a config written for one reads the same in the other:
//!
//! ```lua
//! oslo.prompt.left = {
//!   oslo.segment({
//!     name = "cwd",
//!     priority = 50,
//!     render = function(ctx)
//!       return { { text = oslo.path.shorten(ctx.cwd, 3), style = "prompt.cwd" } }
//!     end,
//!   }),
//! }
//! ```
//!
//! `render` returns a list of **spans** — `{ text = ..., style = ... }` — rather than a string, so
//! the styling stays data. A style is a *name*, looked up in the theme, which is what lets a colour
//! scheme change without touching the prompt.
//!
//! **Priority is the interesting field.** It is what a narrow terminal needs: when the pieces do
//! not fit, the lowest priority goes first, and the prompt degrades in the order the user chose
//! instead of being truncated mid-word.

use super::util::native;
use crate::lua::eval::LuaError;
use crate::lua::eval::value::Value;
use std::rc::Rc;

/// The key a segment table is tagged with, so a bare table cannot be mistaken for one.
///
/// hexe rejects raw segment tables for the same reason: a typo in a field name would otherwise
/// produce a segment that renders nothing, silently.
const TAG: &str = "__oslo_segment";

/// One piece of a prompt, already rendered.
pub struct Rendered {
    /// Named so a config can be told which piece was dropped, and so the tests read as prompts
    /// rather than as widths.
    #[allow(dead_code)]
    pub name: String,
    pub priority: i64,
    /// The spans, each with its style resolved to an escape sequence.
    pub text: String,
    /// Cells the text occupies once escapes are discounted.
    pub width: usize,
}

/// Build the `oslo.segment` constructor.
pub fn constructor() -> Value {
    native("oslo.segment", move |_, args| {
        let Some(Value::Table(spec)) = args.first() else {
            return Err(LuaError::new("oslo.segment: expects a table".to_string()));
        };
        let borrowed = spec.borrow();
        // Checked here rather than at render time: a segment with no `render` is a config mistake,
        // and finding out at the next keystroke — as a prompt that is quietly missing a piece — is
        // how a typo survives for weeks.
        if !matches!(borrowed.get(&Value::str("render")), Value::Function(_)) {
            return Err(LuaError::new(
                "oslo.segment: `render` must be a function".to_string(),
            ));
        }
        if !matches!(borrowed.get(&Value::str("name")), Value::Str(_)) {
            return Err(LuaError::new(
                "oslo.segment: `name` must be a string".to_string(),
            ));
        }
        drop(borrowed);
        // Tagged in place rather than copied: this is the table the config wrote, and Lua's own
        // constructors work the same way — what comes back is what went in, marked.
        spec.borrow_mut().set(Value::str(TAG), Value::Bool(true));
        Ok(vec![Value::Table(Rc::clone(spec))])
    })
}

/// Whether a value is a segment built by [`constructor`].
pub fn is_segment(value: &Value) -> bool {
    match value {
        Value::Table(t) => matches!(t.borrow().get(&Value::str(TAG)), Value::Bool(true)),
        _ => false,
    }
}

/// Whether a value is a *list of* segments, which is how a prompt is configured.
pub fn is_segment_list(value: &Value) -> bool {
    let Value::Table(t) = value else {
        return false;
    };
    let t = t.borrow();
    let len = t.length();
    // An empty list is not a segment list: it is an empty table, which is far more likely to be a
    // config that has not finished being written than a prompt with nothing in it.
    len > 0 && (1..=len).all(|i| is_segment(&t.get(&Value::int(i))))
}

/// The name and priority of a segment, with the defaults hexe uses.
pub fn describe(segment: &Value) -> (String, i64) {
    let Value::Table(t) = segment else {
        return (String::new(), 0);
    };
    let t = t.borrow();
    let name = match t.get(&Value::str("name")) {
        Value::Str(s) => s.to_string(),
        _ => String::new(),
    };
    // Unspecified priority sits in the middle, so a segment that says nothing is dropped after the
    // ones that asked to go first and before the ones that asked to stay.
    let priority = match t.get(&Value::str("priority")) {
        Value::Number(n) => n.as_int().unwrap_or(50),
        _ => 50,
    };
    (name, priority)
}

/// The `render` function of a segment.
pub fn render_fn(segment: &Value) -> Option<Value> {
    let Value::Table(t) = segment else {
        return None;
    };
    let f = t.borrow().get(&Value::str("render"));
    matches!(f, Value::Function(_)).then_some(f)
}

/// Turn what a `render` returned into text, resolving each span's style name.
///
/// Accepts either a list of spans or a bare string, because a segment that just wants to say one
/// unstyled thing should not have to spell out a one-element list.
pub fn spans_to_text(value: &Value, style_of: &dyn Fn(&str, &str) -> String) -> String {
    match value {
        Value::Str(s) => s.to_string(),
        Value::Table(t) => {
            let t = t.borrow();
            let mut out = String::new();
            for i in 1..=t.length() {
                let span = t.get(&Value::int(i));
                match &span {
                    // A plain string in the list is unstyled text.
                    Value::Str(s) => out.push_str(s),
                    Value::Table(fields) => {
                        let fields = fields.borrow();
                        let text = match fields.get(&Value::str("text")) {
                            Value::Str(s) => s.to_string(),
                            Value::Number(n) => n.to_string(),
                            _ => continue,
                        };
                        let style = match fields.get(&Value::str("style")) {
                            Value::Str(s) => s.to_string(),
                            _ => String::new(),
                        };
                        out.push_str(&style_of(&text, &style));
                    }
                    _ => {}
                }
            }
            out
        }
        _ => String::new(),
    }
}

/// Drop the least important segments until the rest fit in `cols`.
///
/// The prompt degrades in the order the config chose. Truncating the joined string instead would
/// cut whichever piece happened to be last, usually mid-word, and usually the one saying where you
/// are — which is the piece a narrow terminal makes most valuable, not least.
pub fn fit(mut pieces: Vec<Rendered>, cols: usize) -> Vec<Rendered> {
    let mut total: usize = pieces.iter().map(|p| p.width).sum();
    while total > cols && pieces.len() > 1 {
        // Lowest priority first; among equals, the later one, so an earlier segment outranks a
        // later one of the same priority and the prompt shortens from the right.
        let Some((at, _)) = pieces
            .iter()
            .enumerate()
            .min_by_key(|(i, p)| (p.priority, std::cmp::Reverse(*i)))
        else {
            break;
        };
        total -= pieces[at].width;
        pieces.remove(at);
    }
    pieces
}

#[cfg(test)]
mod tests {
    use super::*;

    fn piece(name: &str, priority: i64, width: usize) -> Rendered {
        Rendered {
            name: name.to_string(),
            priority,
            text: "x".repeat(width),
            width,
        }
    }

    /// The lowest priority goes first, and what is left still fits.
    #[test]
    fn a_narrow_terminal_drops_the_least_important_piece() {
        let pieces = vec![
            piece("user", 10, 8),
            piece("cwd", 90, 20),
            piece("git", 50, 12),
        ];
        let kept = fit(pieces, 32);
        let names: Vec<&str> = kept.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["cwd", "git"], "the least important goes first");
        assert!(kept.iter().map(|p| p.width).sum::<usize>() <= 32);
    }

    /// Something is always kept: a prompt of nothing at all is worse than one that overruns.
    #[test]
    fn the_last_piece_is_never_dropped() {
        let kept = fit(vec![piece("cwd", 1, 400)], 10);
        assert_eq!(kept.len(), 1);
    }

    /// Equal priorities shorten from the right, so the order written is the order kept.
    #[test]
    fn equal_priorities_drop_the_later_one() {
        let kept = fit(
            vec![piece("a", 50, 10), piece("b", 50, 10), piece("c", 50, 10)],
            20,
        );
        let names: Vec<&str> = kept.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["a", "b"]);
    }
}

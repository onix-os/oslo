//! Key bindings the config writes itself.
//!
//! `oslo.keys` maps a key to one of a fixed set of action names. That list can never be long
//! enough — every shell that has lasted grew a way to run *your* code on a keystroke, because the
//! useful bindings are the ones nobody anticipated. fish has `bind` with `commandline`; zsh has ZLE
//! widgets with `$BUFFER` and `$CURSOR`.
//!
//! ```lua
//! oslo.keys["ctrl-s"] = function(line)
//!   return { text = "sudo " .. line.text, cursor = line.cursor + 5 }
//! end
//! ```
//!
//! **Data in, data out.** The handler is given a table describing the line and answers with what
//! the line should become — it does not mutate anything. That is deliberate: a mutation protocol
//! would need the handler to hold a live reference to the editor while Lua runs arbitrary code,
//! including code that opens another prompt. Returning a description has no such window.
//!
//! Answering `nil` means "leave it alone", which is what a binding that only looks at the line
//! wants.

use crate::lua::eval::value::{Table, Value};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// The action name a Lua-valued binding is recorded under.
///
/// `oslo.keys` carries `(key, action)` pairs of plain strings all the way through the settings, so
/// a function-valued entry is recorded as this name and the function itself is kept here. The name
/// is not spellable in a config — it contains a space — so it cannot collide with a real action.
pub const ACTION: &str = "lua handler";

thread_local! {
    /// Key name to handler. Thread-local because a handler is Lua, and Lua is not `Send`; only the
    /// editor's own thread ever runs one.
    static HANDLERS: RefCell<HashMap<String, Value>> = RefCell::new(HashMap::new());
}

/// Record the handler for a key. Replaces any earlier one, as assigning twice in Lua would.
pub fn register(key: &str, handler: Value) {
    HANDLERS.with(|slot| slot.borrow_mut().insert(key.to_string(), handler));
}

/// Forget every handler — used when a config is reloaded, so a key it no longer binds stops firing
/// the handler it used to have.
pub fn clear() {
    HANDLERS.with(|slot| slot.borrow_mut().clear());
}

/// The handler for a key, if the config gave one.
pub fn handler(key: &str) -> Option<Value> {
    HANDLERS.with(|slot| slot.borrow().get(key).cloned())
}

/// What a handler is told about the line.
pub fn line_table(text: &str, cursor: usize) -> Value {
    let mut t = Table::new();
    t.set(Value::str("text"), Value::str(text));
    // Bytes, matching what Lua's own string functions count. Handing over a character index would
    // disagree with `#line.text` the moment anything is not ASCII.
    t.set(Value::str("cursor"), Value::int(cursor as i64));
    // The word the cursor is in, since almost every binding wants it and working it out in Lua
    // means re-implementing the shell's own tokeniser badly.
    let (start, end) = word_bounds(text, cursor);
    t.set(Value::str("word"), Value::str(&text[start..end]));
    t.set(Value::str("word_start"), Value::int(start as i64));
    Value::Table(Rc::new(RefCell::new(t)))
}

/// What the line should become, read out of what a handler answered.
///
/// `None` when the handler answered nothing, or answered something that is not a line — a handler
/// that returns a number by accident should leave the line alone rather than replace it with `4`.
pub fn line_from(answer: &Value) -> Option<(String, Option<usize>)> {
    match answer {
        // A bare string is the whole line: the common case should not need a table.
        Value::Str(text) => Some((text.to_string(), None)),
        Value::Table(t) => {
            let t = t.borrow();
            let Value::Str(text) = t.get(&Value::str("text")) else {
                return None;
            };
            let cursor = match t.get(&Value::str("cursor")) {
                Value::Number(n) => n.as_int().map(|i| i.max(0) as usize),
                _ => None,
            };
            Some((text.to_string(), cursor))
        }
        _ => None,
    }
}

/// The word around `cursor`, as the shell's own splitting sees it.
///
/// Whitespace-separated, which is what a binding rewriting "the word I am on" means. Quoting and
/// expansion are not considered: a handler that needs those has the whole line and the shell's
/// other helpers.
fn word_bounds(text: &str, cursor: usize) -> (usize, usize) {
    let cursor = cursor.min(text.len());
    let start = text[..cursor]
        .rfind(char::is_whitespace)
        .map(|i| i + 1)
        .unwrap_or(0);
    let end = text[cursor..]
        .find(char::is_whitespace)
        .map(|i| cursor + i)
        .unwrap_or(text.len());
    (start, end)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A bare string is the whole line; a table can move the cursor too.
    #[test]
    fn a_handler_can_answer_with_a_string_or_a_table() {
        assert_eq!(
            line_from(&Value::str("sudo ls")),
            Some(("sudo ls".to_string(), None))
        );

        let mut t = Table::new();
        t.set(Value::str("text"), Value::str("sudo ls"));
        t.set(Value::str("cursor"), Value::int(4));
        let answer = Value::Table(Rc::new(RefCell::new(t)));
        assert_eq!(line_from(&answer), Some(("sudo ls".to_string(), Some(4))));
    }

    /// Anything else leaves the line alone. A handler that only inspects the line returns nothing,
    /// and one that returns a number by accident must not replace the line with it.
    #[test]
    fn anything_that_is_not_a_line_leaves_it_alone() {
        assert_eq!(line_from(&Value::Nil), None);
        assert_eq!(line_from(&Value::int(4)), None);
        assert_eq!(line_from(&Value::Bool(true)), None);
        // A table with no `text` is not a line either.
        let empty = Value::Table(Rc::new(RefCell::new(Table::new())));
        assert_eq!(line_from(&empty), None);
    }

    /// The word under the cursor, including when the cursor sits at either end of it.
    #[test]
    fn the_word_under_the_cursor_is_reported() {
        let line = "git commit --amend";
        assert_eq!(
            word_bounds(line, 0),
            (0, 3),
            "at the start of the first word"
        );
        assert_eq!(word_bounds(line, 3), (0, 3), "at its end");
        assert_eq!(word_bounds(line, 8), (4, 10), "inside the second");
        assert_eq!(word_bounds(line, 18), (11, 18), "at the end of the line");
        // A cursor past the end is clamped rather than panicking.
        assert_eq!(word_bounds(line, 999), (11, 18));
    }
}

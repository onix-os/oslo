//! Reading one field out of a config table.
//!
//! Split from `from_lua.rs` because they are a different job: that file knows which setting each
//! Lua name maps to, and these know how to get a number, a flag or a cursor shape out of a table
//! and what to say when the value is not one. Neither half has to know the other's subject.

use crate::matching::Fuzzy;
use oslo_base::value::Value;

pub(super) fn number(table: &oslo_base::value::Table, name: &str) -> Option<i64> {
    table.get(&Value::str(name)).as_number()?.as_int()
}

/// A cursor-shape field, left alone when the config does not mention it.
///
/// A name nothing answers to is reported rather than silently ignored: a cursor that quietly keeps
/// its default looks exactly like oslo not reading the config at all.
pub(super) fn cursor(
    table: &oslo_base::value::Table,
    name: &str,
    slot: &mut crate::vi::Cursor,
    problems: &mut Vec<String>,
) {
    let Value::Str(text) = table.get(&Value::str(name)) else {
        return;
    };
    match crate::vi::Cursor::parse(&text) {
        Some(cursor) => *slot = cursor,
        None => problems.push(format!(
            "oslo.vi.{name}: '{text}' is not a cursor; \
             use block, line or underscore, optionally followed by blink"
        )),
    }
}

/// A boolean field, left alone when the config does not mention it.
///
/// `false` and "absent" have to be told apart, or `descriptions = false` would be indistinguishable
/// from not setting it and could never turn anything off.
pub(super) fn flag(table: &oslo_base::value::Table, name: &str, slot: &mut bool) {
    match table.get(&Value::str(name)) {
        Value::Nil => {}
        value => *slot = value.truthy(),
    }
}

/// Read a `fuzzy` knob, which takes either a boolean or a preset name.
///
/// Both spellings are accepted because both are the obvious thing to write: `fuzzy = true` is what
/// you reach for first, and `fuzzy = "loose"` is what you reach for once you want to tune it. A
/// name nothing answers to is reported rather than ignored — a typo that silently leaves fuzzy
/// matching off looks exactly like the feature not working.
pub(super) fn fuzzy(
    table: &oslo_base::value::Table,
    path: &str,
    slot: &mut Fuzzy,
    problems: &mut Vec<String>,
) {
    match table.get_str("fuzzy") {
        Value::Nil => {}
        Value::Str(name) => match Fuzzy::parse(name.as_ref()) {
            Some(chosen) => *slot = chosen,
            None => problems.push(format!(
                "{path}: '{name}' is not a preset; use off, tight, smart or loose"
            )),
        },
        value => {
            *slot = if value.truthy() {
                Fuzzy::Smart
            } else {
                Fuzzy::Off
            }
        }
    }
}

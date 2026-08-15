//! `ps` and `ls` as row producers.
//!
//! Both already exist as Lua helpers reading `/proc` and the filesystem — the facts are the same
//! facts, so this converts them rather than gathering them twice. Two implementations of "what the
//! processes are" is how a tool starts giving two answers.
//!
//! Neither replaces the external command. A lone `ps` at the prompt is still `/usr/bin/ps`, and
//! that is deliberate: the text face of these tools has to be byte-identical to what a script
//! already expects, and the honest way to be byte-identical to `ps` is to be `ps`. Structure is
//! offered where it costs nothing — when the next stage asks for it.

use crate::data::{Record, Val};
use oslo_lua::value::Value;

/// Rows for a `sh.*` helper that already answers in rows.
///
/// The Lua side builds a list of tables; this is the same data as records, so the column order
/// survives — which it now does, because Lua tables iterate in insertion order.
fn rows_of(value: &Value) -> Vec<Record> {
    let Value::Table(list) = value else {
        return Vec::new();
    };
    let list = list.borrow();
    let mut out = Vec::new();
    for i in 1..=list.length() {
        let Value::Table(row) = list.get(&Value::int(i)) else {
            continue;
        };
        let row = row.borrow();
        let mut record = Record::new();
        for (key, value) in row.pairs() {
            let Value::Str(name) = key else {
                continue;
            };
            record.set(&name, val_of(&value));
        }
        if !record.is_empty() {
            out.push(record);
        }
    }
    out
}

/// A Lua value as a pipeline value.
///
/// A field whose name says it is a size becomes [`Val::Size`], so `where 'size > 1e6'` is
/// arithmetic and `sort-by size` orders by bytes. That is a naming convention rather than a type,
/// and it is worth it: the alternative is every tool re-stating what its own columns mean.
fn val_of(value: &Value) -> Val {
    match value {
        Value::Nil => Val::Null,
        Value::Bool(b) => Val::Bool(*b),
        Value::Number(n) => match n.as_int() {
            Some(i) => Val::Int(i),
            None => Val::Float(n.as_float()),
        },
        Value::Str(s) => Val::Str(s.to_string()),
        _ => Val::Null,
    }
}

/// Mark the columns whose names mean a byte count.
fn as_sizes(mut rows: Vec<Record>, size_columns: &[&str]) -> Vec<Record> {
    for row in &mut rows {
        for name in size_columns {
            if let Some(Val::Int(bytes)) = row.get(name) {
                let bytes = *bytes;
                if bytes >= 0 {
                    row.set(name, Val::Size(bytes as u64));
                }
            }
        }
    }
    rows
}

/// Every process, from `/proc`.
///
/// Read from `/proc` rather than parsed out of `ps` output: which columns `ps` prints differs
/// between implementations and between invocations, so a parser would be guessing at the machine
/// it is running on.
pub fn ps() -> Vec<Record> {
    rows_of(&crate::data::rows::ps_rows())
}

/// A directory listing, or why there is not one.
///
/// **`Result`, like `df::rows`.** `ls_rows` answers an empty table for a directory that does not
/// exist, cannot be read, or is not a directory at all — the same answer it gives for a directory
/// that is genuinely empty. Routed through `Some((0, rows))` that became `ls /nope | length` saying
/// `0` with status 0 and nothing on stderr, where the ordinary `ls` refuses outright. A wrong
/// answer where a refusal would have been survivable is the shape this module warns about.
pub fn ls(args: &[String]) -> Result<Vec<Record>, String> {
    // The same rule the Lua front end uses, from the same place: a leading `-` is a flag, not a
    // directory. Taking it as one made `ls -la | where …` answer an empty listing without a word.
    let path = crate::data::rows::ls_where(args);
    // Asked here because the lister has nowhere to put the answer: it hands back a table either
    // way. One extra `read_dir` on the path that is about to be walked anyway.
    std::fs::read_dir(path).map_err(|e| format!("ls: {path}: {e}"))?;
    Ok(as_sizes(
        rows_of(&crate::data::rows::ls_rows(path)),
        &["size"],
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use oslo_lua::value::Table;
    use std::cell::RefCell;
    use std::rc::Rc;

    fn lua_rows(pairs: &[&[(&str, Value)]]) -> Value {
        let mut list = Table::new();
        for (i, row) in pairs.iter().enumerate() {
            let mut t = Table::new();
            for (name, value) in row.iter() {
                t.set(Value::str(name), value.clone());
            }
            list.set(
                Value::int(i as i64 + 1),
                Value::Table(Rc::new(RefCell::new(t))),
            );
        }
        Value::Table(Rc::new(RefCell::new(list)))
    }

    /// Columns survive the crossing in the order the Lua table had them — which is the reason the
    /// Lua table had to become insertion-ordered first.
    #[test]
    fn column_order_survives_the_conversion() {
        let value = lua_rows(&[&[
            ("name", Value::str("a")),
            ("size", Value::int(10)),
            ("kind", Value::str("file")),
        ]]);
        let rows = rows_of(&value);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].columns(), ["name", "size", "kind"]);
    }

    /// A column named as a size becomes one, so it sorts and compares as a number of bytes.
    #[test]
    fn a_size_column_becomes_a_size() {
        let value = lua_rows(&[&[("name", Value::str("a")), ("size", Value::int(2048))]]);
        let rows = as_sizes(rows_of(&value), &["size"]);
        assert_eq!(rows[0].get("size"), Some(&Val::Size(2048)));
    }
}

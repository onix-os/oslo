//! A pipeline value as Lua sees it, and a Lua value as a pipeline sees it.
//!
//! **One converter each way, because there were three and they disagreed.** A cell crossed here
//! whenever `where` bound a row's columns as globals, whenever `oslo.register_tool` handed a tool
//! its input, and whenever `ps` and `ls` read the rows their Lua helpers had already built — and
//! each of those sites had written the crossing out again:
//!
//! | cell | to a filter | to a Lua tool | from `sh.ps`, `sh.ls` |
//! |---|---|---|---|
//! | [`Val::Bytes`] | its length, a number | a lossy string | — |
//! | [`Val::Error`] | `nil` | the text `error: …` | — |
//! | a Lua table | — | its shape, kept | `nil`, the cell lost |
//! | a Lua byte string | — | `nil`, the cell lost | `nil`, the cell lost |
//!
//! A filter and the tool it feeds are two halves of one pipeline — `blobs \| where '…' \| redact`
//! passed the same cell to both — so they cannot hold different things. Nor can the two directions
//! disagree with each other: a tool that does nothing but hand its input back has to hand back what
//! it was given, and every arm missing from one side was a cell that such a tool destroyed.
//!
//! # The rule
//!
//! A cell reaches Lua as the nearest thing Lua actually has, and never as its *rendering*: a
//! [`Val::Size`] is a count of bytes and a [`Val::Duration`] a count of nanoseconds, so
//! `free < 1e9` is arithmetic. Handing over `4.2G` would make every comparison a string comparison
//! and every filter quietly wrong, which is the whole reason a size is a distinct kind.
//!
//! The two that were in dispute:
//!
//! * [`Val::Bytes`] is a **byte string**, through [`Value::bytes`] — the constructor that keeps text
//!   as text and anything else as the bytes themselves. Lua has byte strings, so there is no reason
//!   to lose the content: `from_utf8_lossy` is the mojibake `Val::Bytes` exists to prevent, and a
//!   length is the blob thrown away.
//! * [`Val::Error`] is a **table** `{ error = "…" }`. It has to be distinguishable from every other
//!   cell, and `nil` and a plain string are not: `nil` is already what an absent column is, and a
//!   string is already what a column of text is — so a filter could not tell a cell that failed
//!   from one that legitimately held that value. It is the shape `to json` gives the same cell.
//!
//! Neither direction round-trips an error: a table comes back as [`Val::Record`], because Lua has
//! no way to say "this is an error cell" that a tool could not also have written by hand.

use crate::data::{Record, Val};
use oslo_base::value::{Table, Value};

/// One cell, as Lua sees it.
pub fn to_lua(value: &Val) -> Value {
    match value {
        Val::Null => Value::Nil,
        Val::Bool(b) => Value::Bool(*b),
        Val::Int(i) => Value::int(*i),
        Val::Float(f) => Value::float(*f),
        Val::Str(s) => Value::str(s),
        Val::Bytes(b) => Value::bytes(b),
        // Values, not renderings: bytes and nanoseconds, so a comparison means what it looks like.
        Val::Size(bytes) => Value::int(*bytes as i64),
        Val::Duration(nanos) | Val::Time(nanos) => Value::int(*nanos),
        Val::List(items) => {
            let mut list = Table::new();
            for (i, item) in items.iter().enumerate() {
                list.set(Value::int(i as i64 + 1), to_lua(item));
            }
            Value::table(list)
        }
        Val::Record(record) => Value::table(record_table(record)),
        Val::Error(message) => {
            let mut failed = Table::new();
            failed.set(Value::str("error"), Value::str(message));
            Value::table(failed)
        }
    }
}

/// A record's columns as a Lua table, in the order the record has them.
///
/// The order is not incidental — it decides what `cols` and the drawn table show, and a tool that
/// hands its input back must not silently reorder it.
pub fn record_table(record: &Record) -> Table {
    let mut row = Table::new();
    for (name, value) in record.columns().iter().zip(record.values()) {
        row.set(Value::str(name), to_lua(value));
    }
    row
}

/// Records as a Lua list of tables — the reverse of [`records_of`].
pub fn rows_value(rows: &[Record]) -> Value {
    let mut list = Table::new();
    for (i, record) in rows.iter().enumerate() {
        list.set(Value::int(i as i64 + 1), Value::table(record_table(record)));
    }
    Value::table(list)
}

/// One Lua value as a cell — the reverse of [`to_lua`].
pub fn from_lua(value: &Value) -> Val {
    match value {
        Value::Nil => Val::Null,
        Value::Bool(b) => Val::Bool(*b),
        Value::Number(n) => match n.as_int() {
            Some(i) => Val::Int(i),
            None => Val::Float(n.as_float()),
        },
        Value::Str(s) => Val::Str(s.to_string()),
        Value::Bytes(bytes) => Val::Bytes(bytes.to_vec()),
        Value::Table(table) => table_of(&table.borrow()),
        // A function or a userdata is not a value a row can hold, and there is nothing honest to
        // turn one into.
        _ => Val::Null,
    }
}

/// A Lua list of tables as records — the reverse of [`rows_value`].
///
/// A row that contributes no named column is dropped rather than kept as an empty record, so a list
/// with a stray number in it does not become a blank line in the drawn table.
pub fn records_of(value: &Value) -> Vec<Record> {
    let Value::Table(list) = value else {
        return Vec::new();
    };
    let list = list.borrow();
    let mut out = Vec::new();
    for i in 1..=list.length() {
        let Value::Table(row) = list.get(&Value::int(i)) else {
            continue;
        };
        let mut record = Record::new();
        for (key, value) in row.borrow().pairs() {
            if let Value::Str(name) = key {
                record.set(&name, from_lua(&value));
            }
        }
        if !record.is_empty() {
            out.push(record);
        }
    }
    out
}

/// A nested table as a list or a record, the way [`to_lua`] would have written it.
///
/// **The two directions have to agree.** `to_lua` gives [`Val::List`] and [`Val::Record`] an arm
/// each, while this side once read every table as a list of rows and kept the first — so `{ x = 1 }`
/// has length zero, the walk never ran, and the cell came back as `{}`. A tool that did nothing but
/// pass a nested row through destroyed it.
///
/// Lua has one type for both, so the length is the only thing that tells them apart.
fn table_of(table: &Table) -> Val {
    if table.length() >= 1 {
        return Val::List(
            (1..=table.length())
                .map(|i| from_lua(&table.get(&Value::int(i))))
                .collect(),
        );
    }
    let mut record = Record::new();
    for (key, value) in table.pairs() {
        if let Value::Str(name) = key {
            record.set(&name, from_lua(&value));
        }
    }
    Val::Record(record)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn as_int(value: &Value) -> Option<i64> {
        match value {
            Value::Number(n) => n.as_int(),
            _ => None,
        }
    }

    fn as_text(value: &Value) -> Option<String> {
        match value {
            Value::Str(s) => Some(s.to_string()),
            _ => None,
        }
    }

    /// A size compares as a number, which is exactly what `ls -lh | sort` cannot do.
    #[test]
    fn a_size_is_arithmetic_in_lua() {
        assert_eq!(as_int(&to_lua(&Val::Size(1024))), Some(1024));
        assert_eq!(
            as_int(&to_lua(&Val::Duration(1_500_000_000))),
            Some(1_500_000_000)
        );
    }

    /// Every column reaches Lua under its own name.
    #[test]
    fn a_record_reaches_lua_as_a_table() {
        let record = Record::from_pairs([
            ("mount", Val::Str("/".into())),
            ("free", Val::Size(500_000_000)),
        ]);
        let Value::Table(row) = to_lua(&Val::Record(record)) else {
            panic!("a record is a table");
        };
        let row = row.borrow();
        assert_eq!(as_text(&row.get_str("mount")).as_deref(), Some("/"));
        assert_eq!(as_int(&row.get_str("free")), Some(500_000_000));
    }

    /// **The blob crosses whole.** One converter answered its length and the other a string that
    /// `from_utf8_lossy` had already replaced every invalid sequence in — a JPEG arrived as either
    /// a number or mojibake, and `Val::Bytes` exists precisely so that neither happens.
    #[test]
    fn bytes_cross_as_bytes() {
        let jpeg = vec![0xff, 0xd8, 0xff, 0xe0];
        assert_eq!(
            to_lua(&Val::Bytes(jpeg.clone())).as_bytes(),
            Some(&jpeg[..])
        );

        // Bytes that are text arrive as text, so an ordinary comparison still works.
        let text = to_lua(&Val::Bytes(b"hello".to_vec()));
        assert_eq!(as_text(&text).as_deref(), Some("hello"));
    }

    /// **A failed cell is visible and cannot be mistaken for a value.** `nil` is what an absent
    /// column already is and a string is what a column of text already is, so neither could be
    /// told apart from a cell that legitimately held it.
    #[test]
    fn an_error_is_a_table_that_says_so() {
        let Value::Table(failed) = to_lua(&Val::Error("stale handle".into())) else {
            panic!("an error cell is a table");
        };
        assert_eq!(
            as_text(&failed.borrow().get_str("error")).as_deref(),
            Some("stale handle")
        );
        assert!(
            !matches!(to_lua(&Val::Error("x".into())), Value::Nil),
            "an error is not the same as an absent column"
        );
    }

    /// A list stays a list, all of it, and nesting survives.
    #[test]
    fn a_list_keeps_its_shape() {
        let Value::Table(list) = to_lua(&Val::List(vec![Val::Int(10), Val::Int(20)])) else {
            panic!("a list is a table");
        };
        let list = list.borrow();
        assert_eq!(list.length(), 2);
        assert_eq!(as_int(&list.get(&Value::int(2))), Some(20));
    }

    /// A list of tables becomes rows, with the columns in the order the table had them — which is
    /// the reason the Lua table had to become insertion-ordered first.
    #[test]
    fn a_lua_list_of_tables_becomes_records() {
        let mut row = Table::new();
        row.set(Value::str("host"), Value::str("a"));
        row.set(Value::str("ip"), Value::str("10.0.0.1"));
        let mut list = Table::new();
        list.set(Value::int(1), Value::table(row));

        let records = records_of(&Value::table(list));
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].columns(), ["host", "ip"]);
        assert_eq!(records[0].get("ip"), Some(&Val::Str("10.0.0.1".into())));
    }

    /// **A nested table keeps its shape.** One reverse converter dropped every table to `Val::Null`
    /// and the other read them all as lists of rows, keeping only the first — so a map cell, whose
    /// length is zero, arrived as `{}`: `nest | to json` printed `"inner": {}` for `{ x = 1 }`.
    #[test]
    fn a_nested_table_keeps_its_shape() {
        let mut inner = Table::new();
        inner.set(Value::str("x"), Value::int(1));
        let Val::Record(record) = from_lua(&Value::table(inner)) else {
            panic!("a map is a record");
        };
        assert_eq!(record.get("x"), Some(&Val::Int(1)));

        let mut list = Table::new();
        for (i, value) in [10, 20].into_iter().enumerate() {
            list.set(Value::int(i as i64 + 1), Value::int(value));
        }
        assert_eq!(
            from_lua(&Value::table(list)),
            Val::List(vec![Val::Int(10), Val::Int(20)])
        );
    }

    /// The two directions agree: what `to_lua` writes, `from_lua` reads back unchanged.
    #[test]
    fn a_nested_value_survives_the_round_trip() {
        let mut record = Record::new();
        record.set("x", Val::Int(1));
        let nested = Val::Record(record);
        assert_eq!(from_lua(&to_lua(&nested)), nested);

        let list = Val::List(vec![Val::Str("a".into()), Val::Int(2)]);
        assert_eq!(from_lua(&to_lua(&list)), list);
    }

    /// **A blob survives a tool that passes it through.** `to_lua` hands over the bytes themselves
    /// rather than a length or a lossy string, so this side has to read them back as bytes — the
    /// arm was missing from both reverse converters, and they arrived as `Val::Null`.
    #[test]
    fn a_blob_survives_the_round_trip() {
        let jpeg = Val::Bytes(vec![0xff, 0xd8, 0xff, 0xe0]);
        assert_eq!(from_lua(&to_lua(&jpeg)), jpeg);

        // Bytes that *are* text are a Lua string, and come back as one: `Value::bytes` keeps the
        // two variants disjoint, so text never reaches `Value::Bytes` in the first place.
        assert_eq!(
            from_lua(&to_lua(&Val::Bytes(b"hello".to_vec()))),
            Val::Str("hello".to_string())
        );
    }
}

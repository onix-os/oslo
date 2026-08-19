//! `oslo.register_tool` — a structured command written in Lua.
//!
//! ```lua
//! oslo.register_tool{
//!   name     = "hosts",
//!   accepts  = "nothing",              -- nothing | bytes | rows | any
//!   produces = "rows",
//!   rows = function(argv)
//!     return { { host = "a", ip = "10.0.0.1" }, { host = "b", ip = "10.0.0.2" } }
//!   end,
//! }
//! ```
//!
//! `hosts | where 'ip:match("^10%.")'` then works like any other tool.
//!
//! **Facts, not rendering.** A tool says what its rows *are*; the shell decides how they are drawn,
//! and when the next stage wants rows it is never drawn at all — so the tool does not pay for a
//! rendering nobody reads. One source of facts and one renderer is the only arrangement in which
//! the two faces cannot disagree, which is the argument `docs/features/structured-pipelines.md` makes.
//!
//! Deliberately the last part of the pipeline work to be built: the Lua surface should be a face on
//! a shape that has already been proven in Rust, not a guess that then constrains it.

use super::util::{native, put};
use oslo_base::value::LuaError;
use oslo_base::value::{Table, Value};
use oslo_shell::data::plan::Shape;
use oslo_shell::data::{Record, Val};
use std::cell::RefCell;
use std::collections::HashMap;

thread_local! {
    /// Name to the `rows` function. Thread-local because a tool is Lua, and only the shell's own
    /// thread ever runs one.
    static TOOLS: RefCell<HashMap<String, Value>> = RefCell::new(HashMap::new());
}

/// Install `oslo.register_tool`.
pub fn install(oslo: &mut Table) {
    put(oslo, "register_tool", |_, args| {
        let Some(Value::Table(spec)) = args.first() else {
            return Err(LuaError::new(
                "oslo.register_tool: expects a table".to_string(),
            ));
        };
        let spec = spec.borrow();
        let Value::Str(name) = spec.get_str("name") else {
            return Err(LuaError::new(
                "oslo.register_tool: `name` must be a string".to_string(),
            ));
        };
        let rows = spec.get_str("rows");
        if !matches!(rows, Value::Function(_)) {
            return Err(LuaError::new(
                "oslo.register_tool: `rows` must be a function".to_string(),
            ));
        }
        let accepts = shape_of(&spec.get_str("accepts"), Shape::Nothing)?;
        let produces = shape_of(&spec.get_str("produces"), Shape::Rows)?;

        // The handler goes into the pipeline's own table as an opaque closure, so that asking
        // "is there a tool called this" does not mean reaching up into the Lua API. See
        // `oslo_shell::data::custom`.
        TOOLS.with(|slot| slot.borrow_mut().insert(name.to_string(), rows.clone()));
        let handler = std::rc::Rc::new(move |argv: &[String], input: Option<&[Record]>| {
            run_rows(&rows, argv, input)
        });
        oslo_shell::data::custom::register(&name, handler);
        oslo_shell::data::tool::register(&name, accepts, produces);
        Ok(vec![Value::Bool(true)])
    });
    // Kept beside the registration so a config can ask what it has declared, which is the only way
    // to tell a tool that failed to register from one whose name was misspelled.
    oslo.set(
        Value::str("tools"),
        native("oslo.tools", |_, _| {
            let mut list = Table::new();
            let mut names: Vec<String> = TOOLS.with(|slot| slot.borrow().keys().cloned().collect());
            names.sort();
            for (i, name) in names.iter().enumerate() {
                list.set(Value::int(i as i64 + 1), Value::str(name));
            }
            Ok(vec![Value::Table(std::rc::Rc::new(RefCell::new(list)))])
        }),
    );
}

fn shape_of(value: &Value, default: Shape) -> Result<Shape, LuaError> {
    match value {
        Value::Nil => Ok(default),
        Value::Str(name) => match name.as_ref() {
            "nothing" => Ok(Shape::Nothing),
            "bytes" => Ok(Shape::Bytes),
            "rows" => Ok(Shape::Rows),
            "any" => Ok(Shape::Any),
            other => Err(LuaError::new(format!(
                "oslo.register_tool: '{other}' is not a shape; they are nothing, bytes, rows and any"
            ))),
        },
        _ => Err(LuaError::new(
            "oslo.register_tool: a shape is a string".to_string(),
        )),
    }
}

/// Call a Lua tool's `rows` function with `argv` and whatever reached it, and read back what it
/// returned.
///
/// The body of the closure handed to [`oslo_shell::data::custom::register`]. The pipeline calls it
/// without knowing it is Lua, which is the whole point of the split.
///
/// **`function(argv)` still works.** Lua ignores arguments it did not declare, so every tool written
/// before rows could arrive keeps running unchanged; a verb declares `function(argv, input)` and
/// gets them.
fn run_rows(
    handler: &Value,
    argv: &[String],
    input: Option<&[Record]>,
) -> Result<Vec<Record>, String> {
    let mut list = Table::new();
    for (i, word) in argv.iter().enumerate() {
        list.set(Value::int(i as i64 + 1), Value::str(word));
    }
    let argv = Value::Table(std::rc::Rc::new(RefCell::new(list)));
    // `nil` rather than an empty table when nothing reached this stage: "I was given nothing" and
    // "I was given no rows" are different questions, and a verb that filters wants to tell them
    // apart before deciding whether it has been misused.
    let given = match input {
        Some(rows) => rows_value(rows),
        None => Value::Nil,
    };

    match crate::lua::engine::call_here(handler, vec![argv, given]) {
        Ok(values) => Ok(records_of(values.first().unwrap_or(&Value::Nil))),
        Err(e) => Err(e.to_string()),
    }
}

/// Records as a Lua list of tables — the reverse of [`records_of`].
///
/// Column order is kept, because a record's order is not incidental: it decides what `cols` and the
/// drawn table show, and a verb that hands its input back must not silently reorder it.
fn rows_value(rows: &[Record]) -> Value {
    let mut list = Table::new();
    for (i, record) in rows.iter().enumerate() {
        let mut row = Table::new();
        for (name, value) in record.columns().iter().zip(record.values()) {
            row.set(Value::str(name), lua_of(value));
        }
        list.set(
            Value::int(i as i64 + 1),
            Value::Table(std::rc::Rc::new(RefCell::new(row))),
        );
    }
    Value::Table(std::rc::Rc::new(RefCell::new(list)))
}

/// One cell as Lua.
///
/// **The typed cells arrive as their number, not as their rendering.** A `Size` reaches Lua as bytes
/// and a `Duration` as nanoseconds, so `where 'size > 1024'` compares numbers; handing over `4.2G`
/// would make every comparison a string comparison and every filter wrong.
fn lua_of(value: &Val) -> Value {
    match value {
        Val::Null => Value::Nil,
        Val::Bool(b) => Value::Bool(*b),
        Val::Int(i) => Value::int(*i),
        Val::Float(f) => Value::float(*f),
        Val::Str(s) => Value::str(s),
        // Lua strings are byte strings, so bytes cross unchanged rather than being lost.
        Val::Bytes(bytes) => Value::str(String::from_utf8_lossy(bytes)),
        Val::Size(bytes) => Value::int(*bytes as i64),
        Val::Duration(nanos) | Val::Time(nanos) => Value::int(*nanos),
        Val::List(values) => {
            let mut list = Table::new();
            for (i, value) in values.iter().enumerate() {
                list.set(Value::int(i as i64 + 1), lua_of(value));
            }
            Value::Table(std::rc::Rc::new(RefCell::new(list)))
        }
        Val::Record(record) => {
            let mut row = Table::new();
            for (name, value) in record.columns().iter().zip(record.values()) {
                row.set(Value::str(name), lua_of(value));
            }
            Value::Table(std::rc::Rc::new(RefCell::new(row)))
        }
        // An error in one cell is text the rest of the row survives, and it must not be mistaken
        // for a value: a filter comparing it sees a string that says what went wrong.
        Val::Error(message) => Value::str(format!("error: {message}")),
    }
}

/// A Lua list of tables as records.
fn records_of(value: &Value) -> Vec<Record> {
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
            if let Value::Str(name) = key {
                record.set(&name, val_of(&value));
            }
        }
        if !record.is_empty() {
            out.push(record);
        }
    }
    out
}

fn val_of(value: &Value) -> Val {
    match value {
        Value::Nil => Val::Null,
        Value::Bool(b) => Val::Bool(*b),
        Value::Number(n) => match n.as_int() {
            Some(i) => Val::Int(i),
            None => Val::Float(n.as_float()),
        },
        Value::Str(s) => Val::Str(s.to_string()),
        Value::Table(table) => table_of(&table.borrow()),
        _ => Val::Null,
    }
}

/// A nested table as a list or a record, the way [`lua_of`] would have written it.
///
/// **The two directions have to agree.** `lua_of` gives `Val::List` and `Val::Record` an arm each,
/// while this side read every table as *a list of rows* and kept the first — so `{ x = 1 }` has
/// length zero, the walk never ran, and the cell came back as `{}`. A tool that did nothing but
/// pass a nested row through destroyed it.
fn table_of(table: &Table) -> Val {
    // A sequence is a list; anything else is a record of its string-keyed fields. Lua has one type
    // for both, so its length is the only thing that tells them apart.
    if table.length() >= 1 {
        return Val::List(
            (1..=table.length())
                .map(|i| val_of(&table.get(&Value::int(i))))
                .collect(),
        );
    }
    let mut record = Record::new();
    for (key, value) in table.pairs() {
        if let Value::Str(name) = key {
            record.set(&name, val_of(&value));
        }
    }
    Val::Record(record)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The four shapes, and a clear refusal for anything else — a typo in `produces` would
    /// otherwise silently make a tool that never passes rows on.
    #[test]
    fn shapes_are_named_and_typos_are_refused() {
        assert!(matches!(
            shape_of(&Value::str("rows"), Shape::Nothing),
            Ok(Shape::Rows)
        ));
        assert!(matches!(
            shape_of(&Value::Nil, Shape::Bytes),
            Ok(Shape::Bytes)
        ));
        assert!(shape_of(&Value::str("row"), Shape::Rows).is_err());
        assert!(shape_of(&Value::int(3), Shape::Rows).is_err());
    }

    /// A list of tables becomes rows, with the columns in the order the table had them.
    #[test]
    fn a_lua_list_of_tables_becomes_records() {
        let mut row = Table::new();
        row.set_str("host", Value::str("a"));
        row.set_str("ip", Value::str("10.0.0.1"));
        let mut list = Table::new();
        list.set(
            Value::int(1),
            Value::Table(std::rc::Rc::new(RefCell::new(row))),
        );

        let records = records_of(&Value::Table(std::rc::Rc::new(RefCell::new(list))));
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].columns(), ["host", "ip"]);
        assert_eq!(records[0].get("ip"), Some(&Val::Str("10.0.0.1".into())));
    }

    /// **A nested table keeps its shape in both directions.** Every table was read as a list of
    /// rows and only its first kept, so a map cell — length zero, so the walk never ran — arrived
    /// as `{}`: `nest | to json` printed `"inner": {}` for `inner = { x = 1 }`.
    #[test]
    fn a_nested_table_keeps_its_shape() {
        let mut inner = Table::new();
        inner.set_str("x", Value::int(1));
        let record = val_of(&Value::Table(std::rc::Rc::new(RefCell::new(inner))));
        let Val::Record(record) = record else {
            panic!("a map is a record, got {record:?}");
        };
        assert_eq!(record.get("x"), Some(&Val::Int(1)));

        // And a sequence is a list, all of it — not just its first element.
        let mut list = Table::new();
        for (i, value) in [10, 20].into_iter().enumerate() {
            list.set(Value::int(i as i64 + 1), Value::int(value));
        }
        let list = val_of(&Value::Table(std::rc::Rc::new(RefCell::new(list))));
        assert_eq!(list, Val::List(vec![Val::Int(10), Val::Int(20)]));
    }

    /// The two directions agree: what `lua_of` writes, `val_of` reads back unchanged.
    #[test]
    fn a_nested_value_survives_the_round_trip() {
        let mut record = Record::new();
        record.set("x", Val::Int(1));
        let nested = Val::Record(record);
        assert_eq!(val_of(&lua_of(&nested)), nested);

        let list = Val::List(vec![Val::Str("a".into()), Val::Int(2)]);
        assert_eq!(val_of(&lua_of(&list)), list);
    }
}

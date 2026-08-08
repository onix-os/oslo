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
//! the two faces cannot disagree, which is the argument `docs/built-in-tools.md` makes.
//!
//! Deliberately the last part of the pipeline work to be built: the Lua surface should be a face on
//! a shape that has already been proven in Rust, not a guess that then constrains it.

use super::util::{native, put};
use oslo_lua::LuaError;
use oslo_lua::value::{Table, Value};
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
        let Value::Str(name) = spec.get(&Value::str("name")) else {
            return Err(LuaError::new(
                "oslo.register_tool: `name` must be a string".to_string(),
            ));
        };
        let rows = spec.get(&Value::str("rows"));
        if !matches!(rows, Value::Function(_)) {
            return Err(LuaError::new(
                "oslo.register_tool: `rows` must be a function".to_string(),
            ));
        }
        let accepts = shape_of(&spec.get(&Value::str("accepts")), Shape::Nothing)?;
        let produces = shape_of(&spec.get(&Value::str("produces")), Shape::Rows)?;

        // The handler goes into the pipeline's own table as an opaque closure, so that asking
        // "is there a tool called this" does not mean reaching up into the Lua API. See
        // `oslo_shell::data::custom`.
        TOOLS.with(|slot| slot.borrow_mut().insert(name.to_string(), rows.clone()));
        let handler = std::rc::Rc::new(move |argv: &[String]| run_rows(&rows, argv));
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

/// Call a Lua tool's `rows` function with `argv`, and read back what it returned.
///
/// The body of the closure handed to [`oslo_shell::data::custom::register`]. The pipeline calls it
/// without knowing it is Lua, which is the whole point of the split.
fn run_rows(handler: &Value, argv: &[String]) -> Result<Vec<Record>, String> {
    let mut list = Table::new();
    for (i, word) in argv.iter().enumerate() {
        list.set(Value::int(i as i64 + 1), Value::str(word));
    }
    let argv = Value::Table(std::rc::Rc::new(RefCell::new(list)));

    match crate::lua::engine::call_here(handler, vec![argv]) {
        Ok(values) => Ok(records_of(values.first().unwrap_or(&Value::Nil))),
        Err(e) => Err(e.to_string()),
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
        Value::Table(_) => Val::Record(records_of(value).into_iter().next().unwrap_or_default()),
        _ => Val::Null,
    }
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
        row.set(Value::str("host"), Value::str("a"));
        row.set(Value::str("ip"), Value::str("10.0.0.1"));
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
}

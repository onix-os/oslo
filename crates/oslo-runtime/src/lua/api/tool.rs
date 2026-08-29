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
use oslo_shell::data::Record;
use oslo_shell::data::lua::{records_of, rows_value};
use oslo_shell::data::plan::Shape;
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
        let handler = std::rc::Rc::new(
            move |argv: &[String], input: Option<&[Record]>, bytes: Option<&str>| {
                run_rows(&rows, argv, input, bytes)
            },
        );
        oslo_shell::data::custom::register(&name, handler);
        oslo_shell::data::tool::register(&name, accepts, produces);
        // **What it produces, so the planner and the menu can use it.** Optional: a tool that does
        // not say is `Unknown`, exactly as `from json` is, and nothing is refused on an `Unknown`.
        // Saying so buys the same two things the built-in producers get — a mistyped column refused
        // before the tool runs, and the names offered on Tab.
        oslo_shell::data::tool::declare_columns(&name, columns_of(&spec.get_str("columns"))?);
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

/// `columns = { "host", "ip" }` — what a tool says its rows will have.
///
/// Absent is `None`, which is "did not say" rather than "has none": nothing is ever refused on a
/// stream whose columns are unknown, so a tool that stays quiet behaves exactly as it always did.
/// A `columns` that is not a list of names is refused by name, for the same reason a typo in
/// `produces` is — a declaration nobody checks is a declaration that can quietly be wrong.
fn columns_of(value: &Value) -> Result<Option<Vec<String>>, LuaError> {
    match value {
        Value::Nil => Ok(None),
        Value::Table(table) => {
            let mut names = Vec::new();
            for entry in table.borrow().sequence() {
                match entry {
                    Value::Str(name) => names.push(name.to_string()),
                    other => {
                        return Err(LuaError::new(format!(
                            "oslo.register_tool: a column name is a {}, which is not a name",
                            other.type_name()
                        )));
                    }
                }
            }
            Ok(Some(names))
        }
        other => Err(LuaError::new(format!(
            "oslo.register_tool: `columns` is a list of names, not a {}",
            other.type_name()
        ))),
    }
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
    bytes: Option<&str>,
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

    // The third argument, for a tool that declared `accepts = "bytes"`. `nil` for every other
    // shape, by the same rule as `given` above: a tool that was handed no bytes and a tool that
    // never asked for any are different situations.
    //
    // **This copies the whole stream into a Lua string**, which is what declaring `"bytes"` asks
    // for. A tool reading a 200 MB pipe costs 200 MB; one that wants to stream should take `rows`
    // from `lines` in front of it instead.
    let raw = match bytes {
        Some(text) => Value::str(text),
        None => Value::Nil,
    };

    match crate::lua::engine::call_here(handler, vec![argv, given, raw]) {
        Ok(values) => Ok(records_of(values.first().unwrap_or(&Value::Nil))),
        Err(e) => Err(e.to_string()),
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
}

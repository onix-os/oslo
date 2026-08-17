//! `oslo.math` — the calculator, from Lua.
//!
//! ```lua
//! oslo.math.eval("5 km in miles")     --> 3.10685596119, "3.10685596119 miles"
//! oslo.math.value("1 GiB in MB")      --> 1073.741824
//! oslo.math.convert(100, "km/h", "mph")
//! oslo.math.units()                   --> every unit it knows
//!
//! local s = oslo.math.session()       -- remembers what it is told
//! s:eval("r = 3")
//! s:eval("pi * r^2")                  --> 28.274333882308138
//! ```
//!
//! # Two shapes, because there are two questions
//!
//! `eval` answers everything it knows about a result — the number, the rendered text, the unit,
//! the kind — as a table, because a caller building a prompt segment wants the text and a caller
//! doing arithmetic wants the number. `value` answers the number alone, for the common case where
//! the rest would be thrown away.
//!
//! A **session** is the same thing with a memory: variables assigned in one call are there in the
//! next. It is a handle rather than a global so two independent callers cannot tread on each
//! other's names — see [`super::handle`].

use super::util::{native, ok};
use oslo_base::value::{LuaError, Table, Value};
use std::cell::RefCell;
use std::rc::Rc;

/// Build the `oslo.math` table.
pub fn build() -> Table {
    let mut math = Table::new();
    math.set_str(
        "eval",
        native("oslo.math.eval", |_, args| answer(args, false)),
    );
    math.set_str(
        "value",
        native("oslo.math.value", |_, args| answer(args, true)),
    );
    math.set_str(
        "convert",
        native("oslo.math.convert", |_, args| {
            let number = number_at(&args, 0, "oslo.math.convert")?;
            let from = text_at(&args, 1, "oslo.math.convert")?;
            let into = text_at(&args, 2, "oslo.math.convert")?;
            // Built as source rather than by reaching into the engine, so the one grammar decides
            // what a unit is — `convert(1, "km/h", "mph")` and `1 km/h in mph` cannot disagree.
            let source = format!("{number} {from} in {into}");
            match oslo_math::calculate(&source) {
                Ok(found) => ok(Value::float(found.number)),
                Err(why) => Err(LuaError::new(format!("oslo.math.convert: {why}"))),
            }
        }),
    );
    math.set_str(
        "units",
        native("oslo.math.units", |_, _| {
            let mut names: Vec<&str> = oslo_math::units::UNITS.iter().map(|u| u.name).collect();
            names.sort_unstable();
            names.dedup();
            ok(Value::table(list_of(names.into_iter().map(Value::str))))
        }),
    );
    math.set_str(
        "functions",
        native("oslo.math.functions", |_, _| {
            let mut listed = Table::new();
            for (name, about) in oslo_math::functions::NAMES {
                listed.set_str(name, Value::str(about));
            }
            ok(Value::table(listed))
        }),
    );
    math.set_str("session", native("oslo.math.session", |_, _| ok(session())));
    math
}

/// One expression, with no memory. `just_the_number` picks `value` over `eval`.
fn answer(args: Vec<Value>, just_the_number: bool) -> oslo_base::value::LuaResult<Vec<Value>> {
    let source = text_at(&args, 0, "oslo.math.eval")?;
    let mut scope = oslo_math::Scope::new();
    finish(
        oslo_math::calculate_in(&source, &mut scope),
        just_the_number,
    )
}

/// Render an outcome the way whichever entry point asked for.
///
/// **A failure is `nil, message` rather than a raise**, which is the convention everywhere in
/// `oslo.*`: a calculator is asked things that turn out not to parse, and a prompt segment that
/// dies because somebody's expression had a typo is worse than one that shows nothing.
fn finish(
    outcome: Result<oslo_math::Answer, String>,
    just_the_number: bool,
) -> oslo_base::value::LuaResult<Vec<Value>> {
    match outcome {
        Err(why) => Ok(vec![Value::Nil, Value::str(why)]),
        Ok(found) if just_the_number => ok(Value::float(found.number)),
        Ok(found) => {
            let mut out = Table::new();
            out.set_str("value", Value::float(found.number));
            out.set_str("text", Value::str(&found.text));
            out.set_str("unit", Value::str(&found.unit));
            out.set_str("kind", Value::str(&found.dimension));
            ok(Value::table(out))
        }
    }
}

/// A calculator that remembers what it has been told.
fn session() -> Value {
    let scope = Rc::new(RefCell::new(oslo_math::Scope::new()));
    let mut handle = super::handle::Handle::new("oslo.math.session");

    let held = Rc::clone(&scope);
    handle.verb("eval", move |_, args| {
        let source = text_at(&args, 1, "oslo.math.session:eval")?;
        let mut scope = held.borrow_mut();
        finish(oslo_math::calculate_in(&source, &mut scope), false)
    });

    let held = Rc::clone(&scope);
    handle.verb("value", move |_, args| {
        let source = text_at(&args, 1, "oslo.math.session:value")?;
        let mut scope = held.borrow_mut();
        finish(oslo_math::calculate_in(&source, &mut scope), true)
    });

    let held = Rc::clone(&scope);
    handle.verb("names", move |_, _| {
        let scope = held.borrow();
        let mut names: Vec<&String> = scope.names.keys().collect();
        names.sort();
        ok(Value::table(list_of(names.into_iter().map(Value::str))))
    });

    let held = Rc::clone(&scope);
    handle.verb("forget", move |_, _| {
        held.borrow_mut().names.clear();
        ok(Value::Bool(true))
    });

    handle.shows("oslo.math.session");
    handle.build()
}

/// The `at`th argument as text, or a diagnostic naming what it was instead.
fn text_at(args: &[Value], at: usize, who: &str) -> oslo_base::value::LuaResult<String> {
    match args.get(at) {
        Some(Value::Str(text)) => Ok(text.to_string()),
        Some(other) => Err(LuaError::new(format!(
            "{who}: expected an expression, got {}",
            other.type_name()
        ))),
        None => Err(LuaError::new(format!("{who}: an expression is required"))),
    }
}

fn number_at(args: &[Value], at: usize, who: &str) -> oslo_base::value::LuaResult<f64> {
    match args.get(at) {
        Some(Value::Number(n)) => Ok(n.as_float()),
        Some(other) => Err(LuaError::new(format!(
            "{who}: expected a number, got {}",
            other.type_name()
        ))),
        None => Err(LuaError::new(format!("{who}: a number is required"))),
    }
}

/// A Lua sequence from an iterator: `t[1]`, `t[2]`, and so on.
fn list_of(values: impl Iterator<Item = Value>) -> Table {
    let mut out = Table::new();
    for (index, value) in values.enumerate() {
        out.set(Value::int(index as i64 + 1), value);
    }
    out
}

//! What a position offers, as a config writes it.
//!
//! ```lua
//! positional = {
//!   { "dev", "staging\tthe shared one", "$files([.yaml])" },   -- a value list
//!   function(ctx) return sh.lines("git branch --format '%(refname:short)'") end,
//! }
//! ```
//!
//! The list is carapace's, string for string, so a spec written here and a spec read from a file
//! mean the same thing. The function is oslo's, and it is not an escape hatch: a config is written
//! in a language that has functions, and the string DSL exists because YAML has not.

use oslo_base::value::{Table, Value};
use oslo_ui::spec::Action;
use oslo_ui::spec::action::{Offer, Query};
use std::rc::Rc;

/// One position's answer, from whatever Lua put there.
pub fn action(value: &Value) -> Action {
    match value {
        Value::Table(list) => {
            let entries: Vec<String> = list.borrow().sequence().iter().filter_map(one).collect();
            match entries.is_empty() {
                true => Action::None,
                false => Action::List(entries),
            }
        }
        function @ Value::Function(_) => {
            let function = function.clone();
            Action::Call(Rc::new(
                move |query: &Query| match crate::lua::engine::call_here(
                    &function,
                    vec![context(query)],
                ) {
                    Ok(answers) => offers(answers.first()),
                    Err(problem) => {
                        oslo_base::messages::say(
                            oslo_base::messages::Level::Error,
                            "completion/spec".to_string(),
                            problem.to_string(),
                        );
                        Vec::new()
                    }
                },
            ))
        }
        _ => Action::None,
    }
}

/// A list of positions: `positional = { {…}, {…} }`.
pub fn actions(value: &Value) -> Vec<Action> {
    let Value::Table(list) = value else {
        return Vec::new();
    };
    list.borrow().sequence().iter().map(action).collect()
}

/// One entry of a value list.
///
/// **A table is accepted as well as a string**, because `{ value = "dev", desc = "…" }` is what
/// somebody writes before they have read that carapace separates the two with a tab — and the two
/// forms collapse to the same string here rather than to two shapes downstream.
fn one(value: &Value) -> Option<String> {
    match value {
        Value::Str(text) => Some(text.to_string()),
        Value::Table(entry) => {
            let entry = entry.borrow();
            let name = super::string(&entry, "value")
                .or_else(|| super::string(&entry, "display"))
                .or_else(|| entry.sequence().first().and_then(as_text))?;
            Some(
                match super::string(&entry, "desc").or_else(|| super::string(&entry, "description"))
                {
                    Some(description) => format!("{name}\t{description}"),
                    None => name,
                },
            )
        }
        _ => None,
    }
}

fn as_text(value: &Value) -> Option<String> {
    match value {
        Value::Str(text) => Some(text.to_string()),
        _ => None,
    }
}

/// What a computed position answered with: strings, or tables with a description and a tag.
fn offers(value: Option<&Value>) -> Vec<Offer> {
    let Some(Value::Table(list)) = value else {
        return Vec::new();
    };
    list.borrow()
        .sequence()
        .iter()
        .filter_map(|entry| match entry {
            Value::Str(text) => Some(split(text.as_ref())),
            Value::Table(table) => {
                let table = table.borrow();
                let name = super::string(&table, "value")
                    .or_else(|| super::string(&table, "display"))
                    .or_else(|| table.sequence().first().and_then(as_text))?;
                Some(Offer {
                    value: name,
                    description: super::string(&table, "desc")
                        .or_else(|| super::string(&table, "description"))
                        .or_else(|| table.sequence().get(1).and_then(as_text)),
                    tag: super::string(&table, "tag").or_else(|| super::string(&table, "kind")),
                })
            }
            _ => None,
        })
        .collect()
}

/// A returned string is read the way a declared one is: `value\tdescription`.
fn split(text: &str) -> Offer {
    let mut fields = text.splitn(3, '\t');
    Offer {
        value: fields.next().unwrap_or_default().to_string(),
        description: fields.next().filter(|d| !d.is_empty()).map(str::to_string),
        tag: None,
    }
}

/// What a computed position is told about the line.
fn context(query: &Query) -> Value {
    let mut table = Table::new();
    table.set_str("value", Value::str(&query.value));
    table.set_str("dir", Value::str(&query.dir));
    table.set(
        Value::str("args"),
        crate::lua::api::util::list(query.args.iter().map(Value::str)),
    );
    table.set(
        Value::str("words"),
        crate::lua::api::util::list(query.words.iter().map(Value::str)),
    );
    let mut flags = Table::new();
    for (name, value) in &query.flags {
        flags.set_str(name, Value::str(value));
    }
    table.set_str("flags", Value::table(flags));
    Value::table(table)
}

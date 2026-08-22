//! `oslo.completion.provider` — dropdown candidates written in Lua.
//!
//! ```lua
//! oslo.completion.provider {
//!   name = "tldr",
//!   kind = "example",          -- the badge, and the name `oslo.completion.sources` filters on
//!   when = "git",              -- this command only; omit for every command
//!   score_offset = 20,         -- a nudge in the ranking, not a position above it
//!   max_items = 10,
//!   answer = function(ctx)     -- ctx = { command, words, current, arg, cwd }
//!     return { { display = "git commit --amend", desc = "change the last commit" } }
//!   end,
//! }
//! ```
//!
//! # It adds, where `for_command` replaces
//!
//! `oslo.completion.for_command.git` means *I own git*: oslo's own candidates for that command are
//! dropped. A provider merges instead, which is what tldr wants — three examples *beside* the
//! subcommands oslo already knows, competing in one ranking rather than replacing them.

use super::util::{ok, put};
use oslo_base::value::{Table, Value};
use std::rc::Rc;

/// Add `provider` to the `oslo.completion` table.
pub fn install(completion: &mut Table) {
    put(completion, "provider", |_, args| {
        let Some(Value::Table(declared)) = args.first() else {
            return Err(oslo_base::value::LuaError::new(
                "oslo.completion.provider: one table, as in { name = \"tldr\", answer = f }"
                    .to_string(),
            ));
        };
        let declared = declared.borrow();
        let Some(name) = string(&declared, "name") else {
            return Err(oslo_base::value::LuaError::new(
                "oslo.completion.provider: `name` is what this provider is called".to_string(),
            ));
        };
        let answer @ Value::Function(_) = declared.get_str("answer") else {
            return Err(oslo_base::value::LuaError::new(
                "oslo.completion.provider: `answer` must be a function of one argument".to_string(),
            ));
        };

        let named = name.clone();
        oslo_ui::completion::provider::register(oslo_ui::completion::provider::Provider {
            // A provider that declares no kind is badged with its own name, which is always more
            // use than the nothing `for_command` reports — and is a name `sources` can filter on.
            kind: string(&declared, "kind").unwrap_or_else(|| name.clone()),
            when: string(&declared, "when"),
            score_offset: number(&declared, "score_offset").unwrap_or(0.0),
            max_items: number(&declared, "max_items")
                .filter(|n| *n > 0.0)
                .map(|n| n as usize)
                .unwrap_or(oslo_ui::completion::provider::DEFAULT_MAX_ITEMS),
            min_chars: number(&declared, "min_chars")
                .map(|n| n as usize)
                .unwrap_or(0),
            enabled: predicate(&declared),
            name,
            answer: Rc::new(move |ctx| {
                match crate::lua::engine::call_here(&answer, vec![context(ctx)]) {
                    Ok(values) => offers(values.first()),
                    Err(problem) => {
                        oslo_base::messages::say(
                            oslo_base::messages::Level::Error,
                            format!("completion/{named}"),
                            problem.to_string(),
                        );
                        Vec::new()
                    }
                }
            }),
        });
        ok(Value::Bool(true))
    });

    put(completion, "providers", |_, _| {
        ok(super::util::list(
            oslo_ui::completion::provider::names()
                .into_iter()
                .map(Value::str),
        ))
    });
}

/// Read a list of offers out of what Lua answered.
///
/// **Two shapes, because both are natural.** A table with `display` is the full form; a bare string
/// is what somebody writes when there is nothing to say about it, and refusing that would make the
/// simple case the wordy one.
fn offers(value: Option<&Value>) -> Vec<oslo_ui::completion::provider::Offer> {
    let Some(Value::Table(list)) = value else {
        return Vec::new();
    };
    let list = list.borrow();
    (1..)
        .map(|index| list.get(&Value::int(index)))
        .take_while(|value| !matches!(value, Value::Nil))
        .filter_map(|value| match value {
            Value::Str(text) => Some(oslo_ui::completion::provider::Offer {
                display: text.to_string(),
                description: None,
            }),
            Value::Table(entry) => {
                let entry = entry.borrow();
                Some(oslo_ui::completion::provider::Offer {
                    display: string(&entry, "display")?,
                    description: string(&entry, "desc").or_else(|| string(&entry, "description")),
                })
            }
            _ => None,
        })
        .collect()
}

fn context(ctx: &oslo_ui::completion::provider::Ctx) -> Value {
    let mut table = Table::new();
    table.set_str("command", Value::str(&ctx.command));
    table.set_str("current", Value::str(&ctx.current));
    table.set_str("arg", Value::int(ctx.arg as i64));
    table.set_str("cwd", Value::str(&ctx.cwd));
    table.set(
        Value::str("words"),
        super::util::list(ctx.words.iter().map(Value::str)),
    );
    Value::table(table)
}

fn string(table: &Table, key: &str) -> Option<String> {
    match table.get(&Value::str(key)) {
        Value::Str(text) => Some(text.to_string()),
        _ => None,
    }
}

/// `enabled = function(ctx) … end` — the context rule, as a predicate.
///
/// **A function rather than a `when = { … }` table.** A predicate is strictly more expressive than
/// any set of fields would be, and it is written in the language the rest of the config is. A
/// predicate that raises is read as *no*: a guard nobody can evaluate has not said yes.
fn predicate(declared: &Table) -> Option<oslo_ui::completion::provider::Enabled> {
    let asked @ Value::Function(_) = declared.get_str("enabled") else {
        return None;
    };
    Some(Rc::new(move |ctx| {
        match crate::lua::engine::call_here(&asked, vec![context(ctx)]) {
            Ok(values) => values.first().is_some_and(Value::truthy),
            Err(_) => false,
        }
    }))
}

fn number(table: &Table, key: &str) -> Option<f64> {
    match table.get(&Value::str(key)) {
        Value::Number(n) => Some(n.as_float()),
        _ => None,
    }
}

pub(in crate::lua::api) mod forget;

#[cfg(test)]
#[path = "complete/tests.rs"]
mod tests;

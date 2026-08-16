//! `oslo.completion.spec` — declaring what to offer after a command, instead of computing it.
//!
//! ```lua
//! oslo.completion.spec {
//!   command = "notes",
//!   desc = "notes kept in the shell",
//!   subcommands = {
//!     { name = "new",  desc = "start one" },
//!     { name = "list", desc = "every note",
//!       flags = { { "--since", desc = "only newer than" } } },
//!   },
//!   flags = { { "-v", "--verbose", desc = "say more" } },
//! }
//! ```
//!
//! # Why this and not `for_command`
//!
//! `oslo.completion.for_command.notes = function(words) … end` already existed, and it works — but
//! it hands you the words and wishes you luck. Every plugin that wants what `git` gets has to
//! re-implement the same three things: which subcommand am I in, is this word a flag, and what is
//! the description. The shell already does all three for the four specs compiled into it; the only
//! reason a config could not have one was that `CommandSpec` held `&'static str`.
//!
//! fish's `complete -c` is the same idea and is most of why its completions are worth having. This
//! is narrower on purpose: a spec is *data*, so it cannot look at the filesystem, run a command, or
//! decide anything at Tab time. When that is what you need, `for_command` is still there, and the
//! two compose — a spec answers the shape of the command, a function answers what is on the machine.
//!
//! # What it deliberately does not take
//!
//! No `takes = "duration"` on a flag. Nothing in oslo completes the *argument* to a flag yet, so a
//! field that recorded the type would be a promise the Tab key does not keep.

use super::util::{ok, put};
use oslo_base::value::{Table, Value};
use oslo_ui::spec::{CommandSpec, OptionSpec, SubcommandSpec};

/// Add `spec` to the `oslo.completion` table.
pub fn install(completion: &mut Table) {
    put(completion, "spec", |_, args| {
        let Some(Value::Table(declared)) = args.first() else {
            return Err(oslo_base::value::LuaError::new(
                "oslo.completion.spec: one table, as in { command = \"notes\", … }".to_string(),
            ));
        };
        let declared = declared.borrow();
        // `command` rather than `name`, because the table's other `name`s are the subcommands' and
        // two meanings of one key in nested tables is how a config file becomes guesswork.
        let Some(command) = string(&declared, "command") else {
            return Err(oslo_base::value::LuaError::new(
                "oslo.completion.spec: `command` is which command this describes".to_string(),
            ));
        };
        oslo_ui::spec::custom::register(CommandSpec {
            name: command,
            description: string(&declared, "desc")
                .or_else(|| string(&declared, "description"))
                .unwrap_or_default(),
            subcommands: subcommands(&declared),
            options: flags(&declared),
        });
        ok(Value::Bool(true))
    });
}

fn string(table: &Table, key: &str) -> Option<String> {
    match table.get(&Value::str(key)) {
        Value::Str(text) => Some(text.to_string()),
        _ => None,
    }
}

/// Every entry of a sequence field, as tables. Anything that is not a table is skipped rather than
/// refused: one malformed entry in a long list should cost that entry, not the whole spec.
fn entries(table: &Table, key: &str) -> Vec<std::rc::Rc<std::cell::RefCell<Table>>> {
    let Value::Table(list) = table.get(&Value::str(key)) else {
        return Vec::new();
    };
    let list = list.borrow();
    (1..)
        .map(|index| list.get(&Value::int(index)))
        .take_while(|value| !matches!(value, Value::Nil))
        .filter_map(|value| match value {
            Value::Table(entry) => Some(entry),
            _ => None,
        })
        .collect()
}

fn subcommands(table: &Table) -> Vec<SubcommandSpec> {
    entries(table, "subcommands")
        .into_iter()
        .filter_map(|entry| {
            let entry = entry.borrow();
            Some(SubcommandSpec {
                name: string(&entry, "name")?,
                description: string(&entry, "desc")
                    .or_else(|| string(&entry, "description"))
                    .unwrap_or_default(),
                // Recursive, so `docker compose up` is expressible. There is no depth limit here
                // because a Lua table cannot be infinitely deep without being cyclic, and a cyclic
                // one cannot be built by the literal syntax anybody writes a spec in.
                subcommands: subcommands(&entry),
                options: flags(&entry),
            })
        })
        .collect()
}

/// A flag's spellings.
///
/// **The array part is the names**, so `{ "-v", "--verbose", desc = "say more" }` reads the way the
/// flag is written. `{ name = "--verbose" }` is accepted too, because that is what somebody who has
/// just read the `subcommands` shape will type first.
fn flags(table: &Table) -> Vec<OptionSpec> {
    entries(table, "flags")
        .into_iter()
        .filter_map(|entry| {
            let entry = entry.borrow();
            let mut names: Vec<String> = (1..)
                .map(|index| entry.get(&Value::int(index)))
                .take_while(|value| !matches!(value, Value::Nil))
                .filter_map(|value| match value {
                    Value::Str(text) => Some(text.to_string()),
                    _ => None,
                })
                .collect();
            if let Some(name) = string(&entry, "name") {
                names.push(name);
            }
            if names.is_empty() {
                return None;
            }
            Some(OptionSpec {
                names,
                description: string(&entry, "desc")
                    .or_else(|| string(&entry, "description"))
                    .unwrap_or_default(),
            })
        })
        .collect()
}

#[cfg(test)]
#[path = "spec/tests.rs"]
mod tests;

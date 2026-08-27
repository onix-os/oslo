//! `oslo.completion.spec` — declaring what to offer after a command, instead of computing it.
//!
//! ```lua
//! oslo.completion.spec {
//!   command = "notes",
//!   desc = "notes kept in the shell",
//!   flags = { { "-v", "--verbose", desc = "say more" } },
//!   persistent = { ["--store="] = "which database" },
//!   subcommands = {
//!     { name = "new",  desc = "start one", positional = { { "$files([.md])" } } },
//!     { name = "list", desc = "every note", aliases = { "ls" },
//!       flags = { { "--since=", desc = "only newer than",
//!                   values = { "today", "week", "month" } } } },
//!   },
//! }
//! ```
//!
//! # Why this and not `for_command`
//!
//! `oslo.completion.for_command.notes = function(words) … end` already existed, and it works — but
//! it hands you the words and wishes you luck. Every plugin that wants what `git` gets has to
//! re-implement the same four things: which subcommand am I in, is this word a flag, does the flag
//! before it take a value, and what is the description.
//!
//! # The shape is [carapace-spec](https://github.com/carapace-sh/carapace-spec)'s
//!
//! Field for field and modifier for modifier, so that a spec written here, a spec generated from a
//! clap program and a spec read from a `.yaml` are the same object. What is oslo's own is that a
//! position may be a **function** as well as a value list: a config is written in a language that
//! has functions, and the string macros exist because YAML has not.
//!
//! | carapace | here |
//! |---|---|
//! | `name` | `command` at the top, `name` on a subcommand |
//! | `persistentflags` | `persistent` |
//! | `completion.positional` | `positional` |
//! | `completion.flag.<name>` | `values` on the flag itself |
//! | `run`, `group` | not read — see `docs/features/completion-and-matching.md` |

mod flags;
mod values;

use super::util::{ok, put};
use oslo_base::value::{Table, Value};
use oslo_ui::spec::{CommandSpec, OptionSpec, Parsing, SubcommandSpec};

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
        let mut spec = command_from(&declared);
        spec.name = command;
        oslo_ui::spec::custom::register(spec);
        ok(Value::Bool(true))
    });

    put(completion, "specs", |_, _| {
        ok(super::util::list(
            oslo_ui::spec::custom::declared()
                .into_iter()
                .map(Value::str),
        ))
    });
}

/// One command — the top of a spec, or any subcommand of one. The same fields either way.
fn command_from(table: &Table) -> CommandSpec {
    CommandSpec {
        name: string(table, "name").unwrap_or_default(),
        aliases: strings(table, "aliases"),
        description: string(table, "desc")
            .or_else(|| string(table, "description"))
            .unwrap_or_default(),
        hidden: table.get_str("hidden").truthy(),
        parsing: parsing(table),
        subcommands: subcommands(table),
        options: option_specs(table, "flags"),
        persistent: {
            let mut inherited = option_specs(table, "persistent");
            // carapace's own key, so a spec transcribed from a file keeps working.
            inherited.extend(option_specs(table, "persistentflags"));
            inherited
        },
        positional: values::actions(&table.get_str("positional")),
        positional_any: values::action(&table.get_str("positional_any")),
        dash: values::actions(&table.get_str("dash")),
        dash_any: values::action(&table.get_str("dash_any")),
    }
}

fn subcommands(table: &Table) -> Vec<SubcommandSpec> {
    let Value::Table(list) = table.get_str("subcommands") else {
        return Vec::new();
    };
    list.borrow()
        .sequence()
        .iter()
        .filter_map(|value| match value {
            // Anything that is not a table is skipped rather than refused: one malformed entry in
            // a long list should cost that entry, not the whole spec.
            Value::Table(entry) => {
                let entry = entry.borrow();
                let spec = command_from(&entry);
                (!spec.name.is_empty()).then_some(spec)
            }
            _ => None,
        })
        .collect()
}

/// Every flag under one key, in both of the shapes a config may write them.
///
/// The array part is a list of declarations; the map part is carapace's `["-f, --file="] = "desc"`.
/// A table may hold both, and often does — the terse form for the flags with nothing to say and the
/// full one for the flag that has values.
fn option_specs(table: &Table, key: &str) -> Vec<OptionSpec> {
    let Value::Table(list) = table.get_str(key) else {
        return Vec::new();
    };
    let list = list.borrow();
    let mut out: Vec<OptionSpec> = list
        .sequence()
        .iter()
        .filter_map(|value| match value {
            Value::Table(entry) => flags::from_entry(&entry.borrow()),
            // `flags = { "--verbose" }` — a flag with no description is still a flag.
            Value::Str(text) => flags::from_pair(text.as_ref(), &Value::str("")),
            _ => None,
        })
        .collect();
    for (name, value) in list.pairs() {
        if let Value::Str(name) = name
            && let Some(flag) = flags::from_pair(name.as_ref(), &value)
        {
            out.push(flag);
        }
    }
    out
}

fn parsing(table: &Table) -> Parsing {
    match string(table, "parsing").as_deref() {
        Some("non-interspersed") => Parsing::NonInterspersed,
        Some("disabled") => Parsing::Disabled,
        _ => Parsing::Interspersed,
    }
}

fn string(table: &Table, key: &str) -> Option<String> {
    match table.get(&Value::str(key)) {
        Value::Str(text) => Some(text.to_string()),
        _ => None,
    }
}

fn strings(table: &Table, key: &str) -> Vec<String> {
    let Value::Table(list) = table.get_str(key) else {
        return Vec::new();
    };
    list.borrow()
        .sequence()
        .iter()
        .filter_map(|value| match value {
            Value::Str(text) => Some(text.to_string()),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
#[path = "spec/tests.rs"]
mod tests;

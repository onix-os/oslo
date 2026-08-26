//! A parsed spec file, as oslo's completion model.
//!
//! The mapping is field for field, and the fields it does **not** read are listed here rather than
//! passed over in silence, because a spec that half-works is worse than one that says what it left
//! out:
//!
//! | in the file | here |
//! |---|---|
//! | `name`, `aliases`, `description`, `hidden`, `parsing` | the same |
//! | `flags`, `persistentflags` | `options`, `persistent` |
//! | `completion.flag.<name>` | `values` on the flag that name belongs to |
//! | `completion.positional`, `positionalany`, `dash`, `dashany` | the same |
//! | `commands` | `subcommands` |
//! | `run` | **not read** — a spec here describes a command, it does not become one |
//! | `exclusiveflags` | **not read** — nothing is offered differently for it yet |
//! | `group`, `documentation`, `examples` | **not read** — the dropdown has no room for them |

use super::yaml::Node;
use oslo_ui::spec::{Action, CommandSpec, OptionSpec, Parsing, SubcommandSpec, flag};

/// Read one spec file's text.
pub fn spec(source: &str) -> Result<CommandSpec, String> {
    let node = super::yaml::parse(source)?;
    let spec = command(&node);
    match spec.name.is_empty() {
        true => Err("a spec needs a `name`".to_string()),
        false => Ok(spec),
    }
}

fn command(node: &Node) -> CommandSpec {
    let completion = node.get("completion");
    let mut options = flags(node, "flags");
    let mut persistent = flags(node, "persistentflags");
    // `completion.flag` keys on the longhand of a flag declared above it, so the two halves are
    // joined here — after both are read, and once, rather than by searching the tree per keystroke.
    if let Some(values) = completion.and_then(|c| c.get("flag")) {
        attach(&mut options, values);
        attach(&mut persistent, values);
    }

    CommandSpec {
        // carapace allows a usage line as the name — `usage [-F file] profile` — of which the
        // first word is the command and the rest is documentation for a reader.
        name: text(node, "name")
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .to_string(),
        aliases: list(node, "aliases"),
        description: text(node, "description"),
        hidden: node.get("hidden").is_some_and(Node::truthy),
        parsing: match node.get("parsing").and_then(Node::text) {
            Some("non-interspersed") => Parsing::NonInterspersed,
            Some("disabled") => Parsing::Disabled,
            _ => Parsing::Interspersed,
        },
        subcommands: subcommands(node),
        options,
        persistent,
        positional: actions(completion, "positional"),
        positional_any: action(completion.and_then(|c| c.get("positionalany"))),
        dash: actions(completion, "dash"),
        dash_any: action(completion.and_then(|c| c.get("dashany"))),
    }
}

fn subcommands(node: &Node) -> Vec<SubcommandSpec> {
    let Some(commands) = node.get("commands") else {
        return Vec::new();
    };
    commands
        .items()
        .into_iter()
        .map(command)
        .filter(|spec| !spec.name.is_empty())
        .collect()
}

/// Every flag under one key. The value is a description, or a table carrying `nargs` and `default`.
fn flags(node: &Node, key: &str) -> Vec<OptionSpec> {
    let Some(map) = node.get(key) else {
        return Vec::new();
    };
    map.pairs()
        .into_iter()
        .filter_map(|(declaration, value)| {
            let (names, modifiers) = flag::parse(declaration);
            if names.is_empty() {
                return None;
            }
            Some(OptionSpec {
                names,
                description: match value {
                    Node::Scalar(text) => text.clone(),
                    other => other
                        .get("description")
                        .and_then(Node::text)
                        .unwrap_or_default()
                        .to_string(),
                },
                takes: modifiers.takes,
                nargs: nargs(value),
                repeatable: modifiers.repeatable,
                hidden: modifiers.hidden,
                required: modifiers.required,
                default: value
                    .get("default")
                    .and_then(Node::text)
                    .map(str::to_string),
                values: Action::None,
            })
        })
        .collect()
}

fn nargs(value: &Node) -> oslo_ui::spec::Nargs {
    use oslo_ui::spec::Nargs;
    match value
        .get("nargs")
        .and_then(Node::text)
        .map(str::parse::<i64>)
    {
        Some(Ok(-1)) => Nargs::Any,
        Some(Ok(n)) if n > 1 => Nargs::Exactly(n as usize),
        _ => Nargs::One,
    }
}

/// Give each flag the values `completion.flag` declared for it.
fn attach(options: &mut [OptionSpec], values: &Node) {
    for option in options {
        let Some(key) = flag::key(&option.names) else {
            continue;
        };
        if let Some(declared) = values.get(&key) {
            option.values = action(Some(declared));
        }
    }
}

/// A list of positions: `positional: [[…], […]]`.
fn actions(completion: Option<&Node>, key: &str) -> Vec<Action> {
    let Some(list) = completion.and_then(|c| c.get(key)) else {
        return Vec::new();
    };
    list.items()
        .into_iter()
        .map(|item| action(Some(item)))
        .collect()
}

/// One position's value list.
fn action(node: Option<&Node>) -> Action {
    let Some(node) = node else {
        return Action::None;
    };
    let values: Vec<String> = node
        .items()
        .into_iter()
        .filter_map(Node::text)
        .map(str::to_string)
        .collect();
    match values.is_empty() {
        true => Action::None,
        false => Action::List(values),
    }
}

fn text(node: &Node, key: &str) -> String {
    node.get(key)
        .and_then(Node::text)
        .unwrap_or_default()
        .to_string()
}

fn list(node: &Node, key: &str) -> Vec<String> {
    node.get(key)
        .map(Node::items)
        .unwrap_or_default()
        .into_iter()
        .filter_map(Node::text)
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
#[path = "read/tests.rs"]
mod tests;

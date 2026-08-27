//! Built-in completion specs, one module per command.
//!
//! These are hand-written descriptions of each command's subcommands and flags. Adding a
//! command means adding a module here and a line in `all()`.
//!
//! They are written through the three helpers below rather than as struct literals. A
//! [`CommandSpec`] carries a dozen fields now that a position can declare what it completes to,
//! and a spec that only wants a name, a description and some flags should not have to say `None`
//! to the other nine.

pub mod cargo;
pub mod docker;
pub mod git;
pub mod npm;

use super::{CommandSpec, OptionSpec, SubcommandSpec};

/// Every built-in spec, in registration order.
pub(crate) fn all() -> Vec<CommandSpec> {
    vec![git::spec(), cargo::spec(), docker::spec(), npm::spec()]
}

/// Every spelling of one flag, owned.
///
/// A helper rather than `.into()` per element, because a flag has one to three spellings and
/// `vec!["-m".into(), "--message".into()]` is three times the punctuation for the same fact.
pub(crate) fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|name| (*name).to_string()).collect()
}

/// One flag: its spellings and what it is for.
pub(crate) fn opt(spellings: &[&str], description: &str) -> OptionSpec {
    OptionSpec::new(names(spellings), description.to_string())
}

/// One subcommand with flags and nothing under it.
pub(crate) fn sub(name: &str, description: &str, options: Vec<OptionSpec>) -> SubcommandSpec {
    SubcommandSpec {
        name: name.to_string(),
        description: description.to_string(),
        options,
        ..SubcommandSpec::default()
    }
}

/// One subcommand that is itself a group of subcommands — `git remote add`, `docker compose up`.
pub(crate) fn group(
    name: &str,
    description: &str,
    subcommands: Vec<SubcommandSpec>,
) -> SubcommandSpec {
    SubcommandSpec {
        name: name.to_string(),
        description: description.to_string(),
        subcommands,
        ..SubcommandSpec::default()
    }
}

/// A whole command: what `spec()` in each module answers with.
pub(crate) fn command(
    name: &str,
    description: &str,
    subcommands: Vec<SubcommandSpec>,
    options: Vec<OptionSpec>,
) -> CommandSpec {
    CommandSpec {
        name: name.to_string(),
        description: description.to_string(),
        subcommands,
        options,
        ..CommandSpec::default()
    }
}

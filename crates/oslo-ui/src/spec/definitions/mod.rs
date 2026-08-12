//! Built-in completion specs, one module per command.
//!
//! These are hand-written descriptions of each command's subcommands and flags. Adding a
//! command means adding a module here and a line in `all()`.

pub mod cargo;
pub mod docker;
pub mod git;
pub mod npm;

use super::CommandSpec;

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

//! `oslo.nix` — everything nix answers as JSON, as Lua tables.
//!
//! # One function, and names built on it in Lua
//!
//! The tempting shape is a function per useful command: `oslo.nix.metadata()`, `oslo.nix.show()`,
//! `oslo.nix.config()`. Three things say otherwise.
//!
//! **The surface is wide and moves.** Twenty-three subcommands advertise `--json` on nix 2.34.6,
//! and nix's own `--help` opens by saying the interface is subject to change.
//!
//! **Some of those advertisements are false.** `nix registry list --help` documents `--json`;
//! running it answers `error: unrecognised flag '--json'`. Wrappers generated from the help would
//! have shipped a function that cannot work, and a reader would blame oslo.
//!
//! **It would not be extensible.** Names in Rust are names only oslo can add. With one primitive
//! here, `oslo.nix.metadata` is Lua — which means a plugin that wants `closure_size` writes a Lua
//! file rather than a patch to the shell.
//!
//! So this module is [`run`] and [`available`], and everything with a nix-shaped name lives in
//! `lua/nix.lua`.
//!
//! # Thin, like `oslo.direnv`
//!
//! The invocation itself is [`oslo_shell::nix_shell::json`], beside the code that already knows how
//! to talk to nix. What is here is the argument reading and the conversion to Lua.

use super::util::{failed, ok, put};
use oslo_lua::value::{Table, Value};
use oslo_lua::{LuaError, LuaResult};
use oslo_shell::nix_shell::json;
use std::time::Duration;

/// Build the `oslo.nix` table.
pub fn build() -> Value {
    let mut nix = Table::new();

    // oslo.nix.run{"flake", "metadata", timeout = 30} -> table, or nil + message
    put(&mut nix, "run", |_, args| {
        let request = Request::from_lua(args.first())?;
        match json::run(&request.argv, request.timeout) {
            Ok(document) => match serde_json::from_str::<serde_json::Value>(&document) {
                Ok(parsed) => ok(super::json::from_json(&parsed)),
                // nix answered, but not with JSON. `nix fmt --json` does this: the flag is accepted
                // and the output is whatever the formatter printed. A parse error naming the
                // command is more use than a table that is not there.
                Err(e) => failed(
                    &format!("oslo.nix.run: `nix {}`", request.argv.join(" ")),
                    e,
                ),
            },
            Err(message) => Ok(vec![Value::Nil, Value::str(message)]),
        }
    });

    // oslo.nix.available() -> boolean
    put(&mut nix, "available", |_, _| {
        ok(Value::Bool(json::available()))
    });

    Value::table(nix)
}

/// One `oslo.nix.run{…}` request.
#[derive(Debug)]
struct Request {
    argv: Vec<String>,
    timeout: Duration,
}

impl Request {
    /// The table form — positional entries are nix's arguments, named ones are options.
    ///
    /// Deliberately the shape of `oslo.run{…}`, which a config already knows: a list that is passed
    /// through as written, with options beside it rather than in a second argument.
    fn from_lua(value: Option<&Value>) -> LuaResult<Self> {
        let Some(Value::Table(t)) = value else {
            return Err(LuaError::new(format!(
                "oslo.nix.run: expected a table of arguments, got {}",
                value.map_or("no value", Value::type_name)
            )));
        };
        let table = t.borrow();

        let mut argv = Vec::new();
        for (i, entry) in table.sequence().iter().enumerate() {
            argv.push(word(entry).ok_or_else(|| {
                LuaError::new(format!(
                    "oslo.nix.run: argument #{} is a {}, which is not a word",
                    i + 1,
                    entry.type_name()
                ))
            })?);
        }
        if argv.is_empty() {
            return Err(LuaError::new(
                "oslo.nix.run: no arguments — `oslo.nix.run{\"flake\", \"metadata\"}`".to_string(),
            ));
        }

        // Seconds, because that is the unit every number in this neighbourhood is measured in and
        // the ceiling exists for a 46-second command. A non-positive value would mean "kill it
        // before it starts", which nobody means, so it falls back to the default.
        let timeout = match table.get(&Value::str("timeout")).as_number() {
            Some(n) if n.as_float() > 0.0 => Duration::from_secs_f64(n.as_float()),
            _ => json::TIMEOUT,
        };

        Ok(Self { argv, timeout })
    }
}

/// A table entry as one argument, or `None` when it is not something argv can hold.
fn word(value: &Value) -> Option<String> {
    match value {
        Value::Str(s) => Some(s.to_string()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

#[cfg(test)]
#[path = "nix/tests.rs"]
mod tests;

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
use oslo_base::value::{LuaError, LuaResult};
use oslo_base::value::{Table, Value};
use oslo_shell::nix_shell::{cache, json};
use std::time::Duration;

/// The named helpers, in the language they are meant to be replaced in.
const HELPERS: &str = include_str!("nix.lua");

/// Build the `oslo.nix` table.
pub fn build() -> Value {
    let mut nix = Table::new();

    // oslo.nix.run{"flake", "metadata", timeout = 30, cache = true} -> table, or nil + message
    put(&mut nix, "run", |_, args| {
        let request = Request::from_lua(args.first())?;
        match request.answer() {
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

/// Run `nix.lua`, which fills `oslo.nix` in with the named helpers.
///
/// **Called after `oslo` is a global, and the call happens in Lua.** The chunk works by *adding* to
/// the table it is handed, and a table handed across the boundary is a copy — the additions would
/// land on a value thrown away the moment the call returned. Indexing `oslo.nix` from inside the
/// chunk reaches the VM's own table, which is the one every later lookup reads.
///
/// **A failure is reported and startup continues.** These are conveniences over `oslo.nix.run`; a
/// shell that will not start because one of them has a typo is a much worse outcome than a shell
/// missing `oslo.nix.inputs`.
pub(super) fn add_helpers(host: &dyn oslo_luavm::Host) {
    let chunk = "oslo.nix";
    let source = format!("local install = (function() {HELPERS} end)()\ninstall(oslo.nix)");
    if let Err(e) = host.eval(&source, chunk) {
        eprintln!("oslo: {chunk}: {e}");
    }
}

/// One `oslo.nix.run{…}` request.
#[derive(Debug)]
struct Request {
    argv: Vec<String>,
    timeout: Duration,
    cache: bool,
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
        let timeout = match table.get_str("timeout").as_number() {
            Some(n) if n.as_float() > 0.0 => super::util::seconds_to_duration(n.as_float()),
            _ => json::TIMEOUT,
        };

        let cache = table.get_str("cache").truthy();

        Ok(Self {
            argv,
            timeout,
            cache,
        })
    }

    /// The document, from the cache when one was asked for and is still good.
    ///
    /// **Written only after it parses.** A truncated or error-shaped answer kept here is one every
    /// later call is served from, and the caller would have no way to tell why. `nix_shell::apply`
    /// learned the same lesson and guards its cache the same way.
    fn answer(&self) -> Result<String, String> {
        if self.cache
            && let Some(remembered) = cache::document(&self.argv)
        {
            return Ok(remembered);
        }
        let fresh = json::run(&self.argv, self.timeout)?;
        if self.cache && serde_json::from_str::<serde_json::Value>(&fresh).is_ok() {
            cache::keep(&self.argv, &fresh);
        }
        Ok(fresh)
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

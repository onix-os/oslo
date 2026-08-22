//! `oslo.args` — one declaration that both parses and explains itself.
//!
//! ```lua
//! local SPEC = [[
//! # @describe  Put a build somewhere
//! # @option -t --tries <NUM>   how many times to retry
//! # @flag   -n --dry-run       say what would happen
//! # @arg    target!            where to
//! ]]
//!
//! local got, message = oslo.args.parse(SPEC, { "deploy", "--tries", "3", "prod" })
//! --> { tries = "3", target = "prod" }
//! print(oslo.args.usage(SPEC, "deploy"))
//! ```
//!
//! # Why this rather than a second dialect
//!
//! The same argument list gets written three times in an oslo config and shared zero times:
//! `oslo.completion.spec` describes flags for Tab and parses nothing, a recipe's `params` parses and
//! offers nothing, and `register_builtin` is handed raw argv and does neither. The obvious fix is a
//! fourth declaration format that does all three, which is one more than there already are.
//!
//! **argc already is that format.** It parses, it renders `--help`, and it is already what Tab
//! completion reads through `startup/repl/argc.rs`. This is a binding over `argc::eval`, so a
//! recipe and a script written for the `argc` builtin describe their arguments identically.
//!
//! # No shell, on purpose
//!
//! [`oslo_shell::argc::Shell::detached`] is what does the parsing, so nothing here takes the busy
//! lock: this works from a registered builtin, from a completion provider, and from `.make.lua` —
//! which is the entire reason a config wants it. The cost is that a declaration whose default is a
//! shell function (`` --dir=`_default_dir` ``) computes to nothing; a literal default is unaffected.
//!
//! Present only in a build with the `argc` feature, which a config asks for the documented way:
//! `if oslo.args then`. See `docs/features/runtime-features.md`.

use super::util::{list, ok, opt_text, put, text};
use oslo_base::value::{LuaError, Table, Value};
use oslo_shell::argc::Parsed;

/// Build `oslo.args`.
pub fn build() -> Value {
    let mut args = Table::new();
    parsing(&mut args);
    explaining(&mut args);
    Value::table(args)
}

fn parsing(args: &mut Table) {
    // oslo.args.parse(spec, argv) -> table, or nil + message + status
    //
    // A single-valued option is its string; a repeated or multi-valued one is a list, which is the
    // shape it already has in the shell rendering. `--help` and a usage mistake both answer the
    // failure side rather than raising: they are what the caller asked to find out.
    put(args, "parse", |_, call| {
        let spec = text(&call, 1, "oslo.args.parse")?;
        let words = words_of(call.get(1), "oslo.args.parse")?;
        match oslo_shell::argc::parse_words(&spec, &words, None) {
            Err(problem) => Ok(vec![Value::Nil, Value::str(problem), Value::int(1)]),
            Ok(Parsed::Message(text, status)) => Ok(vec![
                Value::Nil,
                Value::str(text),
                Value::int(status as i64),
            ]),
            Ok(Parsed::Values(values)) => {
                let mut out = Table::new();
                for (name, mut values) in values {
                    let key = Value::str(name.replace('-', "_"));
                    if values.len() == 1 {
                        out.set(key, Value::str(values.remove(0)));
                    } else {
                        out.set(key, list(values.into_iter().map(Value::str)));
                    }
                }
                ok(Value::table(out))
            }
        }
    });
}

fn explaining(args: &mut Table) {
    // oslo.args.usage(spec, [name]) -> the help text this declaration renders
    put(args, "usage", |_, call| {
        let spec = text(&call, 1, "oslo.args.usage")?;
        let name = opt_text(&call, 2, "oslo.args.usage")?.unwrap_or_else(|| "command".to_string());
        match oslo_shell::argc::usage_of(&spec, &name, None) {
            Ok(text) => ok(Value::str(text)),
            Err(problem) => Ok(vec![Value::Nil, Value::str(problem)]),
        }
    });

    // oslo.args.check(spec) -> true, or nil + what is wrong with the declaration
    //
    // For a config that builds a declaration rather than writing one out: a mistake in it is
    // otherwise found by the person who runs the command, not by the person who wrote it.
    put(args, "check", |_, call| {
        let spec = text(&call, 1, "oslo.args.check")?;
        match oslo_shell::argc::usage_of(&spec, "command", None) {
            Ok(_) => ok(Value::Bool(true)),
            Err(problem) => Ok(vec![Value::Nil, Value::str(problem)]),
        }
    });
}

/// The argument list, whose first word is the command's own name.
///
/// **Named, not implied.** argc renders `USAGE: <name> …` from `words[0]`, so a caller that passed
/// only the arguments would get help text calling the command by its first flag.
fn words_of(value: Option<&Value>, owner: &str) -> Result<Vec<String>, LuaError> {
    let Some(Value::Table(argv)) = value else {
        return Err(LuaError::new(format!(
            "{owner}: argument #2 must be a list of words, got {}",
            value.map_or("no value", Value::type_name)
        )));
    };
    let mut words = Vec::new();
    for entry in argv.borrow().sequence() {
        match entry {
            Value::Str(word) => words.push(word.to_string()),
            Value::Number(n) => words.push(n.to_string()),
            other => {
                return Err(LuaError::new(format!(
                    "{owner}: an argument is a {}, which is not a word",
                    other.type_name()
                )));
            }
        }
    }
    if words.is_empty() {
        return Err(LuaError::new(format!(
            "{owner}: the list must start with the command's own name"
        )));
    }
    Ok(words)
}

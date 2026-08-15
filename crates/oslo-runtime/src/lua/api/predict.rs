//! `oslo.predict` — what the model thinks comes next, and what a failed line meant.
//!
//! ```lua
//! oslo.predict.next("git ", 3)      -- { { line = "git status", probability = 0.31 }, … }
//! oslo.predict.repair("carg buld")  -- what you probably meant
//! oslo.predict.ready()              -- whether the model has loaded yet
//! ```
//!
//! Both answer a list of `{ line, probability }`, ordered best first and possibly empty — an empty
//! list is the ordinary answer for a shell whose model has not loaded or whose history holds
//! nothing like what was asked.
//!
//! # Why this is enough to build the repair on
//!
//! What to *do* with a candidate — offer it, put it on the line, ask first — is a decision about
//! the interaction, and a config can already express all of it: `oslo.keys` binds a key to a Lua
//! function whose return value replaces the line being edited. With [`install`]'s `oslo.repair`,
//! which asks `$PATH` as well as the model, both halves of `thefuck` are five lines:
//!
//! ```lua
//! oslo.keys["f4"] = function(line)
//!   if line.text == "" then return oslo.repair() or "" end   -- the command that just failed
//!   return oslo.repair(line.text) or line.text               -- the one being typed
//! end
//! ```
//!
//! **Nothing here runs anything**, which is the property that makes a wrong answer cost a keystroke
//! instead of a command: the correction lands in the editor and Enter is still yours.

use super::util::{put, text};
use crate::lua::engine::borrow_env;
use oslo_lua::value::{Number, Table, Value};
use oslo_shell::env::Environment;
use oslo_ui::shell::Shell;
use std::sync::{Arc, Mutex};

/// `oslo.repair(line)` — the corrected line, or nil.
///
/// **Not `oslo.predict.repair`, and the difference is the whole reason it exists.** That one asks
/// the model, which can only ever offer a command you have run. This asks the model *and* `$PATH`:
/// `lsvlk` is a misspelling of a real program whether or not it has ever been typed here, and on a
/// shell with no history the spelling check is the only one of the two that can answer at all.
///
/// This is what a key binding should call. It is the same answer the editor draws after the line.
pub fn install(oslo: &mut Table, env: &Arc<Mutex<Environment>>) {
    // `oslo.last_failed()` — the command you just watched go wrong, or nil.
    put(oslo, "last_failed", |_, _| {
        Ok(vec![
            oslo_base::predict::last_failed()
                .map(Value::str)
                .unwrap_or(Value::Nil),
        ])
    });

    let env = Arc::clone(env);
    put(oslo, "repair", move |_, args| {
        // **No argument means the line that just failed**, which is the whole of what `thefuck`
        // does. A key on the input line can only ever fix what you are still typing; the case
        // people actually reach for is the command already run, and by then it is not on the line
        // to be passed in. Nil rather than an error when nothing failed: `oslo.repair()` at a fresh
        // prompt is a question with an answer, and the answer is "nothing to fix".
        let line = match args.first() {
            None | Some(Value::Nil) => match oslo_base::predict::last_failed() {
                Some(failed) => failed,
                None => return Ok(vec![Value::Nil]),
            },
            _ => text(&args, 1, "oslo.repair")?,
        };
        let guard = borrow_env(&env)?;
        let path = guard.var("PATH").unwrap_or_default().to_string();
        let has = |name: &str| {
            guard.is_builtin(name) || guard.alias(name).is_some() || guard.is_function(name)
        };
        let begins = |stem: &str| {
            guard.builtin_names().any(|n| n.starts_with(stem))
                || guard.aliases().keys().any(|n| n.starts_with(stem))
                || guard.functions().keys().any(|n| n.starts_with(stem))
        };
        let all = || {
            guard
                .builtin_names()
                .map(str::to_string)
                .chain(guard.aliases().keys().cloned())
                .chain(guard.functions().keys().cloned())
                .collect()
        };
        Ok(vec![
            oslo_ui::repair::of(
                &line,
                &path,
                &oslo_ui::repair::Known {
                    has: &has,
                    begins: &begins,
                    all: &all,
                },
            )
            .map(Value::str)
            .unwrap_or(Value::Nil),
        ])
    });
}

/// Build the `oslo.predict` table.
pub fn build() -> Value {
    let mut predict = Table::new();

    // `oslo.predict.next([partial], [limit])`
    put(&mut predict, "next", |_, args| {
        let partial = match args.first() {
            Some(Value::Str(_)) => Some(text(&args, 1, "oslo.predict.next")?),
            _ => None,
        };
        let limit = limit_of(&args, 2);
        Ok(vec![guesses(oslo_base::predict::suggest_here(
            partial.as_deref(),
            limit,
        ))])
    });

    // `oslo.predict.repair(line, [limit])`
    put(&mut predict, "repair", |_, args| {
        let failed = text(&args, 1, "oslo.predict.repair")?;
        let limit = limit_of(&args, 2);
        Ok(vec![guesses(oslo_base::predict::repair_here(
            &failed, limit,
        ))])
    });

    // Whether there is a model at all. A config that wants to say something about it can ask
    // rather than infer it from an empty answer, which is also what "nothing matched" looks like.
    put(&mut predict, "ready", |_, _| {
        Ok(vec![Value::Bool(oslo_base::predict::ready())])
    });

    Value::table(predict)
}

/// How many candidates to ask for. Small by default: this answers a prompt, not a report.
fn limit_of(args: &[Value], at: usize) -> usize {
    match args.get(at.saturating_sub(1)) {
        Some(Value::Number(n)) => (n.as_float() as i64).clamp(1, 32) as usize,
        _ => 3,
    }
}

/// A list of guesses as a Lua list of records.
fn guesses(found: Vec<oslo_base::predict::Guess>) -> Value {
    let mut list = Table::new();
    for (at, guess) in found.into_iter().enumerate() {
        let mut row = Table::new();
        row.set(Value::str("line"), Value::str(guess.line));
        row.set(
            Value::str("probability"),
            Value::Number(Number::Float(guess.probability)),
        );
        list.set(Value::int(at as i64 + 1), Value::table(row));
    }
    Value::table(list)
}

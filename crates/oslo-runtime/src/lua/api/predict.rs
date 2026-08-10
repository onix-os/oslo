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
//! `repair` is the whole of the `thefuck` feature that oslo needs to provide. What to *do* with a
//! candidate — offer it, put it on the line, ask first — is a decision about the interaction, and
//! a config can already express all of it: `oslo.keys` binds a key to a Lua function whose return
//! value replaces the line being edited. So
//!
//! ```lua
//! oslo.keys["f4"] = function(line)
//!   local fixed = oslo.predict.repair(line.text, 1)[1]
//!   return fixed and fixed.line or line.text
//! end
//! ```
//!
//! is a working correction key in four lines, and the line lands in the editor where Enter is
//! still yours. **Nothing here runs anything**, which is the property that makes a wrong answer
//! cost a keystroke instead of a command.

use super::util::{put, text};
use oslo_lua::value::{Number, Table, Value};

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

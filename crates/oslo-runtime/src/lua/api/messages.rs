//! `oslo.messages` — say something the session will remember, and read back what it said.
//!
//! ```lua
//! oslo.messages.warn("notes", "the database moved; the old one is still there")
//! for _, said in ipairs(oslo.messages.all()) do print(said.source, said.text) end
//! ```
//!
//! # Why a plugin needs this and `print` is not it
//!
//! A plugin that finds something wrong at load time has one option today: print it, into a terminal
//! that is one screen of output away from losing it. The `messages` builtin exists so a session's
//! diagnostics can be read after they scroll — and a plugin is the single largest producer of them,
//! so a buffer it cannot write to would keep the shell's own complaints and none of anybody else's.
//!
//! **The source is passed, not guessed.** There is no reliable way to ask "which plugin is running
//! right now" — a handler registered at load time fires long afterwards, from the read loop — and a
//! source attributed to the wrong plugin is worse than one the author typed.

use super::util::{list, ok, put, text};
use oslo_base::messages::{self, Level};
use oslo_lua::value::{Table, Value};

pub fn build() -> Value {
    let mut api = Table::new();

    for (name, level) in [
        ("error", Level::Error),
        ("warn", Level::Warn),
        ("note", Level::Note),
    ] {
        // Each level is its own function rather than a `level = ` argument: `oslo.messages.warn` is
        // what the caller means, and a misspelt level string would silently become a note.
        put(&mut api, name, move |_, args| {
            let source = text(&args, 1, "oslo.messages")?;
            let body = text(&args, 2, "oslo.messages")?;
            match level {
                // The two loud ones print as well, which is what a plugin author expects from
                // something called `warn`. A note is remembered only.
                Level::Error => messages::error(source, body),
                Level::Warn => messages::warn(source, body),
                Level::Note => messages::say(Level::Note, source, body),
            }
            ok(Value::Nil)
        });
    }

    // oslo.messages.all() -> a list of { level, source, text, at, times }
    put(&mut api, "all", |_, _| {
        ok(list(messages::all().into_iter().map(|said| {
            let mut row = Table::new();
            row.set(Value::str("level"), Value::str(said.level.word()));
            row.set(Value::str("source"), Value::str(&said.source));
            row.set(Value::str("text"), Value::str(&said.text));
            row.set(Value::str("at"), Value::float(said.at));
            row.set(Value::str("times"), Value::int(said.times as i64));
            Value::table(row)
        })))
    });

    put(&mut api, "clear", |_, _| {
        messages::clear();
        ok(Value::Nil)
    });

    Value::table(api)
}

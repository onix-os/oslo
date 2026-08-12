//! `oslo.suggest.provider` — a ghost suggestion written in Lua.
//!
//! ```lua
//! oslo.suggest.provider {
//!   name = "tldr",
//!   answer = function(ctx) return "git commit --amend" end,
//! }
//! oslo.suggest.sources = { "history", "provider", "path" }
//! ```
//!
//! # It answers the whole line, not the tail
//!
//! Because that is what a provider naturally has — a page, a model, a table of examples all know
//! *the command*, not the seven characters left to type. The shell computes the remainder, and
//! computing it here is also what lets it check the answer really is one: see the continuation
//! invariant in [`oslo_ui::suggest`].
//!
//! # Where it sits is the config's decision
//!
//! Registering a provider does not put it in front of anything. It is asked when
//! `oslo.suggest.sources` says `provider`, in the position that list gives it — so a plugin cannot
//! decide it outranks the history of what you have really run. VS Code's inline providers work the
//! other way round, with `yieldsToGroupIds` declared by the provider; that is the part not copied.

use super::util::{ok, put, text};
use oslo_lua::value::{Table, Value};
use std::rc::Rc;

/// Add `provider` to the `oslo.suggest` table.
pub fn install(suggest: &mut Table) {
    put(suggest, "provider", |_, args| {
        let Some(Value::Table(declared)) = args.first() else {
            return Err(oslo_lua::LuaError::new(
                "oslo.suggest.provider: one table, as in { name = \"tldr\", answer = f }"
                    .to_string(),
            ));
        };
        let declared = declared.borrow();
        let name = match declared.get(&Value::str("name")) {
            Value::Str(name) => name.to_string(),
            _ => {
                return Err(oslo_lua::LuaError::new(
                    "oslo.suggest.provider: `name` is what this provider is called, and what \
                     `messages` blames when it misbehaves"
                        .to_string(),
                ));
            }
        };
        let answer @ Value::Function(_) = declared.get(&Value::str("answer")) else {
            return Err(oslo_lua::LuaError::new(
                "oslo.suggest.provider: `answer` must be a function of one argument".to_string(),
            ));
        };

        let named = name.clone();
        oslo_ui::suggest::register(oslo_ui::suggest::Provider {
            name,
            answer: Rc::new(move |ctx| {
                let mut table = Table::new();
                table.set(Value::str("line"), Value::str(&ctx.line));
                table.set(Value::str("cursor"), Value::int(ctx.cursor as i64));
                table.set(Value::str("cwd"), Value::str(&ctx.cwd));
                table.set(Value::str("language"), Value::str(&ctx.language));
                // `call_here` rather than a held interpreter: the session's is a thread-local, and
                // capturing an `Rc` to it inside a function stored *in* it would be a cycle that
                // never drops. It is also the same call every other stored handler makes —
                // `register_tool`, `oslo.plugin.test`, the health checks.
                match crate::lua::engine::call_here(&answer, vec![Value::table(table)]) {
                    Ok(values) => match values.first() {
                        Some(Value::Str(line)) => Some(line.to_string()),
                        // Anything else is a decline. A number or a table would be a mistake, but
                        // one made on the keystroke path — reporting it per key would fill the
                        // screen faster than it could be read.
                        _ => None,
                    },
                    Err(problem) => {
                        // Reported and declined. A provider that raises on every keystroke is why
                        // this is a `messages` entry rather than a line on the screen.
                        oslo_base::messages::say(
                            oslo_base::messages::Level::Error,
                            format!("suggest/{named}"),
                            problem.to_string(),
                        );
                        None
                    }
                }
            }),
        });
        ok(Value::Bool(true))
    });

    // oslo.suggest.providers() -> the names registered, in the order they are asked
    put(suggest, "providers", |_, _| {
        ok(super::util::list(
            oslo_ui::suggest::names().into_iter().map(Value::str),
        ))
    });

    // oslo.suggest.forget(name?) -> drop one, or all of them
    put(suggest, "forget", |_, args| {
        match text(&args, 1, "oslo.suggest.forget") {
            Ok(_name) => {
                // One at a time is not offered yet: the registry replaces by name, which covers
                // editing a provider, and nothing has asked to remove exactly one.
                Err(oslo_lua::LuaError::new(
                    "oslo.suggest.forget takes no arguments; it drops every provider".to_string(),
                ))
            }
            Err(_) => {
                oslo_ui::suggest::forget();
                ok(Value::Bool(true))
            }
        }
    });
}

#[cfg(test)]
#[path = "suggest/tests.rs"]
mod tests;

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
        let named = name.clone();
        let ask = match (
            declared.get(&Value::str("answer")),
            declared.get(&Value::str("request")),
        ) {
            (answer @ Value::Function(_), _) => {
                oslo_ui::suggest::Ask::Now(Rc::new(move |ctx| {
                    // `call_here` rather than a held interpreter: the session's is a thread-local,
                    // and capturing an `Rc` to it inside a function stored *in* it would be a cycle
                    // that never drops. It is the same call every other stored handler makes —
                    // `register_tool`, `oslo.plugin.test`, the health checks.
                    match crate::lua::engine::call_here(&answer, vec![context(ctx)]) {
                        Ok(values) => match values.first() {
                            Some(Value::Str(line)) => Some(line.to_string()),
                            // Anything else is a decline. A number or a table would be a mistake,
                            // but one made on the keystroke path — reporting it per key would fill
                            // the screen faster than it could be read.
                            _ => None,
                        },
                        Err(problem) => {
                            complain(&named, &problem);
                            None
                        }
                    }
                }))
            }
            (_, request @ Value::Function(_)) => {
                let debounce = millis(&declared, "debounce_ms", DEFAULT_DEBOUNCE_MS);
                let timeout = millis(&declared, "timeout_ms", DEFAULT_TIMEOUT_MS);
                let for_reply = name.clone();
                oslo_ui::suggest::Ask::Later {
                    request: Rc::new(move |ctx| {
                        // **`reply` is bound to the line it was asked about**, not to whatever is on
                        // screen when it is called. That is what makes a late answer harmless: it
                        // arrives labelled with the question, and the label is what decides whether
                        // it can be drawn.
                        let line = ctx.line.clone();
                        let who = for_reply.clone();
                        let reply = super::util::native("reply", move |_, args| {
                            let said = match args.first() {
                                Some(Value::Str(text)) => Some(text.to_string()),
                                _ => None,
                            };
                            oslo_ui::suggest::answered(&who, &line, said);
                            Ok(vec![Value::Bool(true)])
                        });
                        if let Err(problem) =
                            crate::lua::engine::call_here(&request, vec![context(ctx), reply])
                        {
                            complain(&named, &problem);
                            // Answered with nothing rather than left outstanding: a `request` that
                            // raised is never going to call `reply`, and the editor would poll for
                            // it until the line changed.
                            oslo_ui::suggest::answered(&named, &ctx.line, None);
                        }
                    }),
                    debounce,
                    timeout,
                }
            }
            _ => {
                return Err(oslo_lua::LuaError::new(
                    "oslo.suggest.provider: one of `answer` (fast, answers now) or `request` \
                     (slow, calls reply later) must be a function"
                        .to_string(),
                ));
            }
        };

        oslo_ui::suggest::register(oslo_ui::suggest::Provider { name, ask });
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

/// How long typing must be still before a slow provider is asked.
///
/// Enough that a word typed at speed is one question rather than eight, short enough that an answer
/// still feels like a response to what you just did.
const DEFAULT_DEBOUNCE_MS: u64 = 120;

/// How long an answer may take before it is given up on.
const DEFAULT_TIMEOUT_MS: u64 = 2000;

/// The table a provider is handed.
fn context(ctx: &oslo_ui::suggest::Ctx) -> Value {
    let mut table = Table::new();
    table.set(Value::str("line"), Value::str(&ctx.line));
    table.set(Value::str("cursor"), Value::int(ctx.cursor as i64));
    table.set(Value::str("cwd"), Value::str(&ctx.cwd));
    table.set(Value::str("language"), Value::str(&ctx.language));
    Value::table(table)
}

fn millis(declared: &Table, key: &str, fallback: u64) -> std::time::Duration {
    let asked = match declared.get(&Value::str(key)) {
        Value::Number(n) => n.as_int().filter(|n| *n >= 0).map(|n| n as u64),
        _ => None,
    };
    std::time::Duration::from_millis(asked.unwrap_or(fallback))
}

/// Report once per failure rather than per keystroke: a provider that raises does so on every key.
fn complain(name: &str, problem: &oslo_lua::LuaError) {
    oslo_base::messages::say(
        oslo_base::messages::Level::Error,
        format!("suggest/{name}"),
        problem.to_string(),
    );
}

#[cfg(test)]
#[path = "suggest/tests.rs"]
mod tests;

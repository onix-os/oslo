//! The shell's variables, as Lua's global namespace.
//!
//! This is what makes `export one=two` in shell readable as `one` in Lua on the next line, and
//! `name = "world"` in Lua readable as `$name` in shell. One namespace, two spellings.
//!
//! # The two rules, and why they are that way round
//!
//! **Reading is `_G` first, the shell second.** A shell script that sets `type=deploy` or
//! `print=/usr/bin/lpr` must not break `type()` and `print()` in Lua, and putting the standard
//! library first means it cannot.
//!
//! **Writing sends a string to the shell and keeps everything else in `_G`.** A shell variable can
//! only hold a string; flattening a table to `table: 0x55f…` would lose it. So `name = "world"`
//! crosses over and `handlers = {}` does not.
//!
//! Both rules are a metatable on the globals table: `__index` fires only for a name `_G` does not
//! have, which *is* "the shell second", and `__newindex` fires only for a name `_G` does not have,
//! which is what keeps a script's strings out of `_G` so they stay reachable as shell variables.
//!
//! # Why a script's tables are kept beside `_G` rather than in it
//!
//! Both metamethods fire **only for a name the table does not already have**, and that one fact
//! decides the whole design. Writing `x = {}` straight into `_G` puts the name there for good:
//! `__newindex` never fires for `x` again, so a later `x = "s"` lands in `_G` too and `$x` is
//! never set. That was a real divergence from the evaluator this replaces — the tree walker moved
//! the name — and it was documented here as a thing oslo did not do.
//!
//! So a name a shell variable cannot hold goes into a table *beside* `_G` instead. `_G` never
//! gains the name, `__newindex` keeps firing for it, and the two homes stay exactly one: a string
//! is the shell's, anything else is Lua's, and changing the type moves it either way.
//!
//! **What that costs, stated plainly.** Reading a script's own table global is now a metamethod
//! and a table lookup rather than a raw hit in `_G` — the standard library is untouched, because
//! those names really are in `_G` and neither metamethod fires for them. And two raw operations
//! stop seeing script globals: `rawget(_G, "x")` and `pairs(_G)`, which cannot be intercepted
//! (Lua 5.4 has no `__pairs`). Nothing in oslo enumerates `_G`, and the two places that mention
//! doing so say it must *not* show the shell's internals, which this keeps true.
//!
//! The alternative — an always-empty `_ENV` with everything in a backing table — expresses the
//! rule with no raw-access divergence at all, and costs a metamethod on *every* global read,
//! `print` and `string` included. This is the cheaper half of that trade.

use luna::{Callback, CallbackReturn, Context, String as LunaStr, Table, Value};
use std::rc::Rc;

/// Where a global goes when Lua does not already have it.
pub trait Globals {
    fn get(&self, name: &str) -> Option<String>;
    fn set(&self, name: &str, value: &str);
    fn unset(&self, name: &str);
}

/// Route absent globals through `host`, by giving the globals table a metatable.
pub(crate) fn attach<'gc>(ctx: Context<'gc>, host: Rc<dyn Globals>) {
    let meta = Table::new(&ctx);
    // The names a script assigned something a shell variable cannot hold. Beside `_G`, never in
    // it — see the module docs — and reachable through the metatable, so the collector keeps it.
    let beside = Table::new(&ctx);

    let reader = Rc::clone(&host);
    let index = Callback::from_fn_with(&ctx, beside, move |beside, ctx, _exec, mut stack| {
        let answer = match key_of(&stack) {
            None => Value::Nil,
            Some(name) => {
                // Lua's own first: a script that set `x` to a table means that table, not
                // whatever the shell happens to have under the same name.
                let held = beside.get_value(ctx, LunaStr::from_slice(&ctx, name.as_bytes()));
                match held {
                    Value::Nil => reader.get(&name).map_or(Value::Nil, |text| {
                        Value::String(LunaStr::from_slice(&ctx, text.as_bytes()))
                    }),
                    held => held,
                }
            }
        };
        stack.replace(ctx, answer);
        Ok(CallbackReturn::Return)
    });

    let writer = Rc::clone(&host);
    let newindex = Callback::from_fn_with(&ctx, beside, move |beside, ctx, _exec, mut stack| {
        // `(table, key, value)`, the shape Lua hands `__newindex`.
        let (_table, key, value): (Value, Value, Value) = stack.consume(ctx)?;
        let Some(name) = as_name(key) else {
            return Ok(CallbackReturn::Return);
        };
        let key = LunaStr::from_slice(&ctx, name.as_bytes());
        match value {
            // A string is the one kind of value a shell variable can hold, so it crosses over —
            // and is deliberately *not* written into `_G`, or the next read would find it there
            // and never ask the shell. Whatever Lua was holding under the name is dropped, or the
            // read above would answer with it and the string would be invisible.
            Value::String(text) => {
                let _ = beside.set(ctx, key, Value::Nil);
                writer.set(&name, &String::from_utf8_lossy(text.as_bytes()));
            }
            // `x = nil` removes the name from both homes, which is what "unset" means in each.
            Value::Nil => {
                let _ = beside.set(ctx, key, Value::Nil);
                writer.unset(&name);
            }
            other => {
                writer.unset(&name);
                let _ = beside.set(ctx, key, other);
            }
        }
        Ok(CallbackReturn::Return)
    });

    let _ = meta.set(ctx, "__index", Value::Function(index.into()));
    let _ = meta.set(ctx, "__newindex", Value::Function(newindex.into()));
    ctx.globals().set_metatable(ctx, Some(meta));
}

/// The key `__index` was asked for: `(table, key)`.
fn key_of(stack: &luna::Stack<'_, '_>) -> Option<String> {
    as_name(stack.get(1))
}

/// A key as a name, or `None` when it is not one a shell variable could have.
fn as_name(key: Value<'_>) -> Option<String> {
    match key {
        Value::String(s) => Some(String::from_utf8_lossy(s.as_bytes()).into_owned()),
        _ => None,
    }
}

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
//! **One divergence from the evaluator this replaces**, recorded rather than hidden: a name first
//! assigned a non-string and *then* a string stays in `_G` instead of moving to the shell, because
//! `__newindex` does not fire for a name that is already there. `x = {}` followed by `x = "s"`
//! leaves `$x` unset. The tree walker moved it. Nothing in the shell or its configs does this.

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

    let reader = Rc::clone(&host);
    let index = Callback::from_fn(&ctx, move |ctx, _exec, mut stack| {
        let name = key_of(&stack);
        let answer = name
            .and_then(|name| reader.get(&name))
            .map_or(Value::Nil, |text| {
                Value::String(LunaStr::from_slice(&ctx, text.as_bytes()))
            });
        stack.replace(ctx, answer);
        Ok(CallbackReturn::Return)
    });

    let writer = Rc::clone(&host);
    let newindex = Callback::from_fn(&ctx, move |ctx, _exec, mut stack| {
        // `(table, key, value)`, the shape Lua hands `__newindex`.
        let (table, key, value): (Value, Value, Value) = stack.consume(ctx)?;
        let Some(name) = as_name(key) else {
            return Ok(CallbackReturn::Return);
        };
        match value {
            // A string is the one kind of value a shell variable can hold, so it crosses over —
            // and is deliberately *not* written into `_G`, or the next read would find it there
            // and never ask the shell.
            Value::String(text) => writer.set(&name, &String::from_utf8_lossy(text.as_bytes())),
            // `x = nil` removes the name from both homes, which is what "unset" means in each.
            Value::Nil => writer.unset(&name),
            other => {
                writer.unset(&name);
                if let Value::Table(table) = table {
                    let key = Value::String(LunaStr::from_slice(&ctx, name.as_bytes()));
                    let _ = table.set(ctx, key, other);
                }
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

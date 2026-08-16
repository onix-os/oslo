//! The Lua VM, behind the same front door as the evaluator it replaces.
//!
//! `luna` is a stackless bytecode VM with a tracing collector, in pure Rust. Against the tree walker
//! in `oslo-lua` that is coroutines, `goto`, real string patterns, byte-exact strings, collected
//! cycles and unbounded recursion — measured at 15–30× the speed — with no C anywhere, so a static
//! musl build still needs nothing installed.
//!
//! # The one thing that shapes everything above this
//!
//! luna's values carry a garbage-collector lifetime: `Value<'gc>` exists only inside
//! `lua.enter(|ctx| …)`, and holding one across calls means stashing it in the VM's registry. That
//! is the opposite of `oslo_lua::Value`, which is an `Rc` anyone can keep. So the boundary is real
//! and it lives here: oslo's own value type stays the interchange currency for the ~40 files that
//! have nothing to do with Lua — the structured pipeline, settings, hooks — and is converted at the
//! edge, once, rather than infecting them with a lifetime.

use luna::{Closure, Executor, Lua};

/// Run `source` and answer the exit status, reporting an error the way the shell would.
pub fn run(source: &str, chunk_name: &str) -> i32 {
    let mut lua = Lua::full();
    let executor = match lua.try_enter(|ctx| {
        let closure = Closure::load(ctx, Some(chunk_name), source.as_bytes())?;
        Ok(ctx.stash(Executor::start(ctx, closure.into(), ())))
    }) {
        Ok(executor) => executor,
        Err(error) => {
            eprintln!("oslo: lua: {error}");
            return 1;
        }
    };
    match lua.execute::<()>(&executor) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("oslo: lua: {error}");
            1
        }
    }
}

/// Converting between the shell's value and the VM's, which is the whole boundary.
///
/// oslo's [`oslo_base::value::Value`] is owned and lives anywhere; luna's `Value<'gc>` is a handle
/// into a collected heap and lives only inside `Lua::enter`. Everything the shell hands to Lua goes
/// one way through here, and everything Lua answers comes back the other, so the lifetime stops at
/// this file rather than spreading through forty crates that have no VM in them.
pub mod convert {
    use luna::{Context, String as LunaStr, Table, Value};
    use oslo_base::value::{Number, Value as Own};

    /// The shell's value, as something the VM can hold.
    pub fn into_lua<'gc>(ctx: Context<'gc>, value: &Own) -> Value<'gc> {
        match value {
            Own::Nil => Value::Nil,
            Own::Bool(b) => Value::Boolean(*b),
            // Lua's own split, kept: `3` and `3.0` compare equal but format differently and divide
            // differently, which is why `oslo_base` keeps two variants rather than one `f64`.
            Own::Number(Number::Int(i)) => Value::Integer(*i),
            Own::Number(Number::Float(f)) => Value::Number(*f),
            // Copied, not borrowed: `IntoValue for &str` wants `&'static`, and the shell's strings
            // are `Rc<str>` owned by a value that outlives nothing in particular.
            Own::Str(s) => Value::String(LunaStr::from_slice(&ctx, s.as_bytes())),
            Own::Table(table) => {
                let out = Table::new(&ctx);
                for (key, value) in table.borrow().pairs() {
                    // A `set` only fails on a nil or NaN key. `pairs` yields neither: a nil key
                    // cannot be stored, and a NaN one never compares equal to itself so it could
                    // not have been either.
                    let _ = out.set(ctx, into_lua(ctx, &key), into_lua(ctx, &value));
                }
                Value::Table(out)
            }
            // A function the shell is holding came *from* the VM in the first place; handing back
            // an opaque handle it cannot call would be worse than saying nothing.
            Own::Function(_) => Value::Nil,
        }
    }

    /// The VM's value, as something the shell can keep.
    ///
    /// **Key order is not preserved.** oslo's table remembers the order keys were inserted, which is
    /// what makes `to json` and `to text` print a row's columns the way the document had them.
    /// luna's hash part has no order to give back — it is `ahash`, seeded per process, so two runs
    /// disagree. A row that goes into Lua and comes back out therefore loses its column order. That
    /// is a real gap for the structured pipeline and is recorded in `PLAN-LUA-VM.md`; the array part
    /// (`1..n`) is unaffected, because it is a vector.
    ///
    /// `seen` breaks cycles: `t.self = t` is legal Lua and the shell's table is an `Rc` graph with
    /// no collector, so following it twice would recurse until the stack ran out.
    pub fn from_lua(value: Value<'_>) -> Own {
        from_lua_within(value, 0)
    }

    /// How deep a table may nest before the conversion gives up.
    ///
    /// A depth cap rather than a visited-set, because the shell's values are trees by construction —
    /// what this guards against is a *cyclic* table reaching the conversion at all, and a script can
    /// make one in a line.
    const MAX_DEPTH: usize = 64;

    fn from_lua_within(value: Value<'_>, depth: usize) -> Own {
        match value {
            Value::Nil => Own::Nil,
            Value::Boolean(b) => Own::Bool(b),
            Value::Integer(i) => Own::Number(Number::Int(i)),
            Value::Number(f) => Own::Number(Number::Float(f)),
            Value::String(s) => Own::str(String::from_utf8_lossy(s.as_bytes())),
            Value::Table(t) if depth < MAX_DEPTH => {
                let mut out = oslo_base::value::Table::new();
                for (key, value) in t.iter() {
                    out.set(
                        from_lua_within(key, depth + 1),
                        from_lua_within(value, depth + 1),
                    );
                }
                Own::Table(std::rc::Rc::new(std::cell::RefCell::new(out)))
            }
            // Deeper than the cap, or a thread, a userdata, a function: nothing the shell's own
            // value type has a home for.
            _ => Own::Nil,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::convert::{from_lua, into_lua};
    use luna::Lua;
    use oslo_base::value::{Number, Table, Value};
    use std::cell::RefCell;
    use std::rc::Rc;

    /// A value rendered so that two tables with the same pairs compare equal whatever order they
    /// came back in.
    fn sorted(value: &Value) -> String {
        match value {
            Value::Table(t) => {
                let mut pairs: Vec<String> = t
                    .borrow()
                    .pairs()
                    .iter()
                    .map(|(k, v)| format!("{k:?}={}", sorted(v)))
                    .collect();
                pairs.sort();
                format!("{{{}}}", pairs.join(","))
            }
            other => format!("{other:?}"),
        }
    }

    fn table(pairs: &[(Value, Value)]) -> Value {
        let mut t = Table::new();
        for (k, v) in pairs {
            t.set(k.clone(), v.clone());
        }
        Value::Table(Rc::new(RefCell::new(t)))
    }

    /// **The boundary is the whole port**, so it is asserted rather than assumed: what the shell
    /// hands the VM comes back as the same thing.
    #[test]
    fn a_value_survives_the_round_trip() {
        let mut lua = Lua::core();
        let cases = vec![
            Value::Nil,
            Value::Bool(true),
            Value::Number(Number::Int(-7)),
            Value::Number(Number::Float(0.5)),
            Value::str("hello"),
            // The shape a structured row has: string keys, mixed values.
            table(&[
                (Value::str("name"), Value::str("oslo")),
                (Value::str("size"), Value::Number(Number::Int(42))),
            ]),
            // And the shape a list has: integer keys from one.
            table(&[
                (Value::Number(Number::Int(1)), Value::str("a")),
                (Value::Number(Number::Int(2)), Value::str("b")),
            ]),
        ];

        lua.enter(|ctx| {
            for original in &cases {
                let there = into_lua(ctx, original);
                let back = from_lua(there);
                // Compared as *sets* of pairs, not by rendering: `Value` has no `PartialEq` — Lua's
                // equality is not Rust's — and the hash part comes back in whatever order `ahash`
                // seeded this process with, so an ordered comparison fails at random. See the note
                // on `from_lua`: losing key order is a known gap, not a flaky test.
                assert_eq!(
                    sorted(&back),
                    sorted(original),
                    "round trip changed {original:?}"
                );
            }
        });
    }

    /// An integer and a float are different values in Lua, and the conversion must not quietly
    /// merge them — `string.format("%d", x)` and `1//0` both depend on the difference.
    #[test]
    fn an_integer_does_not_become_a_float() {
        let mut lua = Lua::core();
        lua.enter(|ctx| {
            let there = into_lua(ctx, &Value::Number(Number::Int(3)));
            assert!(matches!(there, luna::Value::Integer(3)));
            let back = from_lua(there);
            assert!(matches!(back, Value::Number(Number::Int(3))), "{back:?}");
        });
    }

    /// A table deep enough to be a cycle is refused rather than followed: the shell's tables are
    /// `Rc` with no collector, so recursing a cyclic one runs until the stack ends.
    #[test]
    fn a_table_too_deep_is_not_followed_for_ever() {
        let mut lua = Lua::core();
        lua.enter(|ctx| {
            let cyclic = luna::Table::new(&ctx);
            cyclic.set(ctx, "self", cyclic).expect("set");
            // The point is that this returns at all.
            let _ = from_lua(luna::Value::Table(cyclic));
        });
    }
}

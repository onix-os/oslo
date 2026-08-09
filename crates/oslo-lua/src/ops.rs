//! Operators, and the metamethods behind them.
//!
//! Lua 5.4's arithmetic has one rule that cannot be simplified away: **integer op integer stays an
//! integer, everything else becomes a float.** `7 // 2` is `3` and `7.0 // 2` is `3.0`; `1 / 2` is
//! `0.5` even though both operands are integers, because `/` is always float. Getting this wrong
//! does not raise an error, it silently changes what a script computes — which is why the subtype
//! is tracked rather than collapsed to `f64`.
//!
//! Every operator falls back to a metamethod when the operands are not numbers or strings, which
//! is what makes `__index`, `__add` and friends work. That fallback is the reason metatables were
//! included in oslo's Lua subset at all: without them, most idiomatic Lua library code stops.

use super::value::{Number, Value};
use super::{Interp, LuaError, LuaResult};
use std::rc::Rc;

/// Look up `name` in `value`'s metatable.
///
/// Strings share one metatable in real Lua, which is how `("x"):upper()` works; here the string
/// library is consulted directly by the index path instead, so only tables carry one.
pub fn metamethod(value: &Value, name: &str) -> Option<Value> {
    let Value::Table(t) = value else {
        return None;
    };
    let meta = t.borrow().metatable.clone()?;
    let found = meta.borrow().get(&Value::str(name));
    match found {
        Value::Nil => None,
        other => Some(other),
    }
}

/// Try a binary metamethod on either operand, in Lua's order: left first, then right.
fn binary_meta(interp: &Interp, name: &str, lhs: &Value, rhs: &Value) -> Option<LuaResult<Value>> {
    let handler = metamethod(lhs, name).or_else(|| metamethod(rhs, name))?;
    Some(
        interp
            .call(&handler, vec![lhs.clone(), rhs.clone()])
            .map(|mut vs| {
                if vs.is_empty() {
                    Value::Nil
                } else {
                    vs.remove(0)
                }
            }),
    )
}

/// The arithmetic operators.
pub fn arith(interp: &Interp, op: &str, lhs: &Value, rhs: &Value) -> LuaResult<Value> {
    // Strings coerce to numbers in arithmetic — `"10" + 1` is 11 in Lua, a wart it keeps for
    // compatibility and that scripts in the wild do rely on.
    if let (Some(a), Some(b)) = (lhs.as_number(), rhs.as_number()) {
        return numeric(op, a, b);
    }

    let event = match op {
        "+" => "__add",
        "-" => "__sub",
        "*" => "__mul",
        "/" => "__div",
        "%" => "__mod",
        "^" => "__pow",
        "//" => "__idiv",
        "&" => "__band",
        "|" => "__bor",
        "~" => "__bxor",
        "<<" => "__shl",
        ">>" => "__shr",
        _ => "__add",
    };
    if let Some(result) = binary_meta(interp, event, lhs, rhs) {
        return result;
    }

    let offender = if lhs.as_number().is_none() { lhs } else { rhs };
    Err(LuaError::new(format!(
        "attempt to perform arithmetic on a {} value",
        offender.type_name()
    )))
}

/// Arithmetic on two numbers, honouring the integer/float rules.
fn numeric(op: &str, a: Number, b: Number) -> LuaResult<Value> {
    let both_int = matches!((a, b), (Number::Int(_), Number::Int(_)));
    let (x, y) = (a.as_float(), b.as_float());

    Ok(match op {
        // Wrapping, not saturating: Lua integers wrap on overflow, and `math.maxinteger + 1`
        // being `math.mininteger` is documented behaviour scripts test for.
        "+" if both_int => Value::int(a.as_int().unwrap().wrapping_add(b.as_int().unwrap())),
        "-" if both_int => Value::int(a.as_int().unwrap().wrapping_sub(b.as_int().unwrap())),
        "*" if both_int => Value::int(a.as_int().unwrap().wrapping_mul(b.as_int().unwrap())),
        "+" => Value::float(x + y),
        "-" => Value::float(x - y),
        "*" => Value::float(x * y),

        // Always float, whatever the operands were.
        "/" => Value::float(x / y),
        "^" => Value::float(x.powf(y)),

        "//" if both_int => {
            let (p, q) = (a.as_int().unwrap(), b.as_int().unwrap());
            if q == 0 {
                return Err(LuaError::new("attempt to perform 'n//0'"));
            }
            Value::int(p.div_euclid(q).wrapping_sub(i64::from(
                p.rem_euclid(q) != 0
                    && (p % q != 0)
                    && ((p < 0) != (q < 0))
                    && p.rem_euclid(q) == 0,
            )))
        }
        "//" => Value::float((x / y).floor()),

        "%" if both_int => {
            let (p, q) = (a.as_int().unwrap(), b.as_int().unwrap());
            if q == 0 {
                return Err(LuaError::new("attempt to perform 'n%%0'"));
            }
            // Lua's `%` takes the sign of the *divisor*, unlike Rust's `%`, so `-1 % 3` is 2.
            Value::int(p.rem_euclid(q) * if q < 0 && p.rem_euclid(q) != 0 { -1 } else { 1 })
        }
        "%" => {
            let r = x - (x / y).floor() * y;
            Value::float(r)
        }

        // Bitwise operators need exact integers; a fractional operand is an error rather than a
        // truncation, because silently dropping the fraction gives a wrong answer.
        "&" | "|" | "~" | "<<" | ">>" => {
            let (Some(p), Some(q)) = (a.as_int(), b.as_int()) else {
                return Err(LuaError::new("number has no integer representation"));
            };
            match op {
                "&" => Value::int(p & q),
                "|" => Value::int(p | q),
                "~" => Value::int(p ^ q),
                "<<" if !(0..64).contains(&q) => Value::int(0),
                "<<" => Value::int(((p as u64) << q) as i64),
                ">>" if !(0..64).contains(&q) => Value::int(0),
                _ => Value::int(((p as u64) >> q) as i64),
            }
        }
        other => return Err(LuaError::new(format!("unknown operator '{other}'"))),
    })
}

/// Unary minus, `#`, `not` and `~`.
pub fn unary(interp: &Interp, op: &str, operand: &Value) -> LuaResult<Value> {
    match op {
        "not" => Ok(Value::Bool(!operand.truthy())),
        "-" => {
            if let Some(n) = operand.as_number() {
                return Ok(match n {
                    Number::Int(i) => Value::int(i.wrapping_neg()),
                    Number::Float(f) => Value::float(-f),
                });
            }
            if let Some(h) = metamethod(operand, "__unm") {
                return one(interp.call(&h, vec![operand.clone(), operand.clone()])?);
            }
            Err(LuaError::new(format!(
                "attempt to perform arithmetic on a {} value",
                operand.type_name()
            )))
        }
        "~" => match operand.as_number().and_then(Number::as_int) {
            Some(i) => Ok(Value::int(!i)),
            None => Err(LuaError::new("number has no integer representation")),
        },
        "#" => match operand {
            Value::Str(s) => Ok(Value::int(s.len() as i64)),
            Value::Table(t) => {
                if let Some(h) = metamethod(operand, "__len") {
                    return one(interp.call(&h, vec![operand.clone()])?);
                }
                Ok(Value::int(t.borrow().length()))
            }
            other => Err(LuaError::new(format!(
                "attempt to get length of a {} value",
                other.type_name()
            ))),
        },
        other => Err(LuaError::new(format!("unknown unary operator '{other}'"))),
    }
}

/// `==`, honouring `__eq` only when both sides are tables and raw equality failed.
pub fn equals(interp: &Interp, lhs: &Value, rhs: &Value) -> LuaResult<bool> {
    if lhs.lua_eq(rhs) {
        return Ok(true);
    }
    if matches!((lhs, rhs), (Value::Table(_), Value::Table(_)))
        && let Some(result) = binary_meta(interp, "__eq", lhs, rhs)
    {
        return Ok(result?.truthy());
    }
    Ok(false)
}

/// `<` and `<=`.
pub fn compare(interp: &Interp, op: &str, lhs: &Value, rhs: &Value) -> LuaResult<bool> {
    // Numbers compare numerically, strings lexicographically, and the two never compare with each
    // other — `1 < "2"` is an error in Lua, not a coercion.
    if let (Value::Number(a), Value::Number(b)) = (lhs, rhs) {
        // **Two integers compare as integers.** Going through `f64` first is lossy above 2^53, so
        // `math.maxinteger - 1 < math.maxinteger` compared two equal floats and answered false.
        if let (Number::Int(x), Number::Int(y)) = (a, b) {
            return Ok(if op == "<" { x < y } else { x <= y });
        }
        let (x, y) = (a.as_float(), b.as_float());
        return Ok(if op == "<" { x < y } else { x <= y });
    }
    if let (Value::Str(a), Value::Str(b)) = (lhs, rhs) {
        return Ok(if op == "<" { a < b } else { a <= b });
    }
    let event = if op == "<" { "__lt" } else { "__le" };
    if let Some(result) = binary_meta(interp, event, lhs, rhs) {
        return Ok(result?.truthy());
    }
    Err(LuaError::new(format!(
        "attempt to compare {} with {}",
        lhs.type_name(),
        rhs.type_name()
    )))
}

/// `..`, which joins strings and numbers and defers everything else to `__concat`.
pub fn concat(interp: &Interp, lhs: &Value, rhs: &Value) -> LuaResult<Value> {
    let joinable = |v: &Value| matches!(v, Value::Str(_) | Value::Number(_));
    if joinable(lhs) && joinable(rhs) {
        return Ok(Value::str(format!(
            "{}{}",
            lhs.to_display(),
            rhs.to_display()
        )));
    }
    if let Some(result) = binary_meta(interp, "__concat", lhs, rhs) {
        return result;
    }
    let offender = if joinable(lhs) { rhs } else { lhs };
    Err(LuaError::new(format!(
        "attempt to concatenate a {} value",
        offender.type_name()
    )))
}

/// `t[k]`, following `__index` chains.
pub fn index(interp: &Interp, target: &Value, key: &Value) -> LuaResult<Value> {
    match target {
        Value::Table(t) => {
            let raw = t.borrow().get(key);
            if !matches!(raw, Value::Nil) {
                return Ok(raw);
            }
            match metamethod(target, "__index") {
                // `__index` as a table means "look there instead", which is how inheritance is
                // spelled in Lua; as a function it is called.
                Some(Value::Function(f)) => {
                    let handler = Value::Function(Rc::clone(&f));
                    one(interp.call(&handler, vec![target.clone(), key.clone()])?)
                }
                Some(other) => index(interp, &other, key),
                None => Ok(Value::Nil),
            }
        }
        // `("abc"):upper()` and `s:sub(1,2)` both land here: strings index into the string library.
        Value::Str(_) => {
            let library = interp.global("string");
            match library {
                Value::Table(t) => Ok(t.borrow().get(key)),
                _ => Ok(Value::Nil),
            }
        }
        other => Err(LuaError::new(format!(
            "attempt to index a {} value",
            other.type_name()
        ))),
    }
}

/// `t[k] = v`, following `__newindex`.
pub fn set_index(interp: &Interp, target: &Value, key: Value, value: Value) -> LuaResult<()> {
    let Value::Table(t) = target else {
        return Err(LuaError::new(format!(
            "attempt to index a {} value",
            target.type_name()
        )));
    };

    // `__newindex` only fires when the key is *absent*; assigning over an existing key is always
    // a raw write. Missing that condition makes proxy tables recurse.
    let present = !matches!(t.borrow().get(&key), Value::Nil);
    if !present && let Some(handler) = metamethod(target, "__newindex") {
        return match handler {
            Value::Function(f) => {
                let h = Value::Function(Rc::clone(&f));
                interp
                    .call(&h, vec![target.clone(), key, value])
                    .map(|_| ())
            }
            other => set_index(interp, &other, key, value),
        };
    }

    if matches!(key, Value::Nil) {
        return Err(LuaError::new("table index is nil"));
    }
    if let Value::Number(n) = &key
        && n.as_float().is_nan()
    {
        return Err(LuaError::new("table index is NaN"));
    }
    t.borrow_mut().set(key, value);
    Ok(())
}

/// `tostring`, honouring `__tostring`.
pub fn tostring(interp: &Interp, value: &Value) -> LuaResult<String> {
    if let Some(handler) = metamethod(value, "__tostring") {
        let result = one(interp.call(&handler, vec![value.clone()])?)?;
        return Ok(result.to_display());
    }
    if let Some(Value::Str(name)) = metamethod(value, "__name")
        && let Value::Table(t) = value
    {
        return Ok(format!("{name}: {:p}", Rc::as_ptr(t)));
    }
    Ok(value.to_display())
}

/// The first value of a call's results, which is what an expression position wants.
fn one(mut values: Vec<Value>) -> LuaResult<Value> {
    Ok(if values.is_empty() {
        Value::Nil
    } else {
        values.remove(0)
    })
}

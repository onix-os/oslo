//! The `math` library.
//!
//! The subtype rules matter more here than the arithmetic does. `math.floor(3.7)` returns the
//! **integer** 3, not `3.0`, because its whole purpose is to produce something usable as a table
//! index; `math.sqrt(4)` returns `2.0`, because every transcendental function is float-valued.

use super::super::value::{Number, Value};
use super::super::{Interp, LuaError, LuaResult};
use super::{arg, arg_int, module, native};
use std::cell::Cell;

pub fn install(interp: &mut Interp) {
    let library = module(vec![
        ("floor", native("math.floor", floor)),
        ("ceil", native("math.ceil", ceil)),
        ("abs", native("math.abs", abs)),
        ("max", native("math.max", max)),
        ("min", native("math.min", min)),
        ("sqrt", float1("math.sqrt", f64::sqrt)),
        ("sin", float1("math.sin", f64::sin)),
        ("cos", float1("math.cos", f64::cos)),
        ("tan", float1("math.tan", f64::tan)),
        ("asin", float1("math.asin", f64::asin)),
        ("acos", float1("math.acos", f64::acos)),
        ("atan", float1("math.atan", f64::atan)),
        ("exp", float1("math.exp", f64::exp)),
        ("log", native("math.log", log)),
        ("fmod", native("math.fmod", fmod)),
        ("modf", native("math.modf", modf)),
        ("tointeger", native("math.tointeger", tointeger)),
        ("type", native("math.type", subtype)),
        ("random", native("math.random", random)),
        ("randomseed", native("math.randomseed", randomseed)),
        ("pi", Value::float(std::f64::consts::PI)),
        ("huge", Value::float(f64::INFINITY)),
        ("maxinteger", Value::int(i64::MAX)),
        ("mininteger", Value::int(i64::MIN)),
    ]);
    interp.set_global("math", library);
}

/// The argument of a one-argument numeric function.
fn number(args: &[Value], function: &str) -> LuaResult<Number> {
    arg(args, 1).as_number().ok_or_else(|| {
        LuaError::new(format!(
            "bad argument #1 to '{function}' (number expected, got {})",
            arg(args, 1).type_name()
        ))
    })
}

/// Wrap an `f64 -> f64` as a Lua function, since most of the library is exactly that.
fn float1(name: &'static str, f: fn(f64) -> f64) -> Value {
    native(name, move |_: &mut Interp, args: Vec<Value>| {
        Ok(vec![Value::float(f(number(&args, name)?.as_float()))])
    })
}

fn floor(_: &mut Interp, args: Vec<Value>) -> LuaResult<Vec<Value>> {
    Ok(vec![match number(&args, "floor")? {
        Number::Int(i) => Value::int(i),
        Number::Float(f) => integral(f.floor()),
    }])
}

fn ceil(_: &mut Interp, args: Vec<Value>) -> LuaResult<Vec<Value>> {
    Ok(vec![match number(&args, "ceil")? {
        Number::Int(i) => Value::int(i),
        Number::Float(f) => integral(f.ceil()),
    }])
}

/// A rounded float as an integer, staying a float when it will not fit one.
fn integral(f: f64) -> Value {
    if f >= i64::MIN as f64 && f <= i64::MAX as f64 {
        Value::int(f as i64)
    } else {
        Value::float(f)
    }
}

fn abs(_: &mut Interp, args: Vec<Value>) -> LuaResult<Vec<Value>> {
    Ok(vec![match number(&args, "abs")? {
        // Wrapping, as Lua's integer arithmetic does throughout: `math.abs(math.mininteger)` is
        // `math.mininteger`, because the positive value has no representation.
        Number::Int(i) => Value::int(i.wrapping_abs()),
        Number::Float(f) => Value::float(f.abs()),
    }])
}

fn max(interp: &mut Interp, args: Vec<Value>) -> LuaResult<Vec<Value>> {
    extremum(interp, args, "max")
}

fn min(interp: &mut Interp, args: Vec<Value>) -> LuaResult<Vec<Value>> {
    extremum(interp, args, "min")
}

fn extremum(_: &mut Interp, args: Vec<Value>, which: &str) -> LuaResult<Vec<Value>> {
    if args.is_empty() {
        return Err(LuaError::new(format!(
            "bad argument #1 to '{which}' (number expected, got no value)"
        )));
    }
    let mut best = arg(&args, 1);
    let mut best_float = number(&args, which)?.as_float();
    for i in 2..=args.len() {
        let candidate = arg(&args, i);
        let value = candidate.as_number().map(Number::as_float).ok_or_else(|| {
            LuaError::new(format!(
                "bad argument #{i} to '{which}' (number expected, got {})",
                candidate.type_name()
            ))
        })?;
        let better = if which == "max" {
            value > best_float
        } else {
            value < best_float
        };
        if better {
            best = candidate;
            best_float = value;
        }
    }
    Ok(vec![best])
}

fn log(_: &mut Interp, args: Vec<Value>) -> LuaResult<Vec<Value>> {
    let x = number(&args, "log")?.as_float();
    Ok(vec![Value::float(match args.get(1) {
        Some(Value::Nil) | None => x.ln(),
        // The common bases are special-cased because `ln(x)/ln(10)` does not give exactly 2 for
        // `log(100, 10)`, and scripts do compare the result.
        _ => match arg(&args, 2).as_number().map(Number::as_float) {
            Some(2.0) => x.log2(),
            Some(10.0) => x.log10(),
            Some(b) => x.ln() / b.ln(),
            None => return Err(LuaError::new("bad argument #2 to 'log' (number expected)")),
        },
    })])
}

fn fmod(_: &mut Interp, args: Vec<Value>) -> LuaResult<Vec<Value>> {
    let x = number(&args, "fmod")?.as_float();
    let y = arg(&args, 2)
        .as_number()
        .map(Number::as_float)
        .ok_or_else(|| LuaError::new("bad argument #2 to 'fmod' (number expected)"))?;
    // C's `fmod`, which takes the sign of the *dividend* — the opposite of Lua's `%` operator.
    Ok(vec![Value::float(x % y)])
}

fn modf(_: &mut Interp, args: Vec<Value>) -> LuaResult<Vec<Value>> {
    let x = number(&args, "modf")?.as_float();
    let whole = x.trunc();
    Ok(vec![integral(whole), Value::float(x - whole)])
}

fn tointeger(_: &mut Interp, args: Vec<Value>) -> LuaResult<Vec<Value>> {
    Ok(vec![match arg(&args, 1) {
        Value::Number(n) => n.as_int().map(Value::int).unwrap_or(Value::Nil),
        // A string is not converted: `math.tointeger("3")` is nil in 5.4, unlike the arithmetic
        // operators, which do coerce.
        _ => Value::Nil,
    }])
}

fn subtype(_: &mut Interp, args: Vec<Value>) -> LuaResult<Vec<Value>> {
    Ok(vec![match arg(&args, 1) {
        Value::Number(Number::Int(_)) => Value::str("integer"),
        Value::Number(Number::Float(_)) => Value::str("float"),
        _ => Value::Nil,
    }])
}

thread_local! {
    /// State for `math.random`.
    ///
    /// A xorshift64* generator rather than a crate: this is for shuffling a list and picking a
    /// colour, not for anything that must resist prediction. Nothing here should be mistaken for
    /// a source of secrets, and a dependency would invite exactly that mistake.
    static SEED: Cell<u64> = const { Cell::new(0x2545_F491_4F6C_DD1D) };
}

fn next_random() -> u64 {
    SEED.with(|seed| {
        let mut x = seed.get();
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        seed.set(x);
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    })
}

fn random(_: &mut Interp, args: Vec<Value>) -> LuaResult<Vec<Value>> {
    let raw = next_random();
    match args.len() {
        // No arguments: a float in [0, 1).
        0 => Ok(vec![Value::float((raw >> 11) as f64 / (1u64 << 53) as f64)]),
        _ => {
            let (low, high) = if args.len() == 1 {
                (1, arg_int(&args, 1, "random")?)
            } else {
                (arg_int(&args, 1, "random")?, arg_int(&args, 2, "random")?)
            };
            if low > high {
                return Err(LuaError::new(
                    "bad argument #2 to 'random' (interval is empty)",
                ));
            }
            let span = (high - low) as u64 + 1;
            Ok(vec![Value::int(low + (raw % span) as i64)])
        }
    }
}

fn randomseed(_: &mut Interp, args: Vec<Value>) -> LuaResult<Vec<Value>> {
    let seed = match args.first() {
        Some(v) => v.as_number().map(|n| n.as_float() as i64).unwrap_or(0),
        // Lua 5.4 seeds from something varying when called with no argument.
        None => std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as i64)
            .unwrap_or(0),
    };
    // Never zero: xorshift is stuck at zero for ever.
    SEED.with(|s| s.set((seed as u64) | 1));
    Ok(Vec::new())
}

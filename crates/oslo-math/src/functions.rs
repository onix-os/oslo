//! The functions an expression can call.
//!
//! # Most of them want a plain number
//!
//! `sin(2 m)` has no meaning: the sine of a length is not a thing. So the ones that take an angle
//! or a ratio refuse a dimensioned argument, and say which one they got. The exceptions are the
//! ones that are *about* magnitude — `abs`, `min`, `max`, `round` — which keep whatever unit came
//! in, and `sqrt`, which halves the dimension when it evenly can.
//!
//! # Angles are radians, and `deg` is how you say otherwise
//!
//! The radian is dimensionless, so `sin(90 deg)` arrives here as `sin(1.5707…)` and works without
//! this module knowing anything about degrees. That is the whole reason angle is not a base
//! dimension — see [`crate::dimension`].

use crate::dimension::Dimension;
use crate::value::Value;

/// Every name, with how many arguments it takes, for `math --functions` and for completion.
pub const NAMES: &[(&str, &str)] = &[
    ("abs", "magnitude, keeping the unit"),
    ("sqrt", "square root; halves the unit's exponents"),
    ("cbrt", "cube root"),
    ("root", "root(x, n) — the nth root"),
    ("exp", "e to the power"),
    ("ln", "natural logarithm"),
    ("log", "log(x) base 10, or log(x, b) base b"),
    ("log2", "logarithm base 2"),
    ("sin", "sine of an angle in radians"),
    ("cos", "cosine"),
    ("tan", "tangent"),
    ("asin", "inverse sine, in radians"),
    ("acos", "inverse cosine"),
    ("atan", "inverse tangent"),
    ("atan2", "atan2(y, x)"),
    ("sinh", "hyperbolic sine"),
    ("cosh", "hyperbolic cosine"),
    ("tanh", "hyperbolic tangent"),
    ("floor", "round down, keeping the unit"),
    ("ceil", "round up"),
    ("round", "round to nearest; round(x, n) to n places"),
    ("trunc", "towards zero"),
    ("sign", "-1, 0 or 1"),
    ("min", "the smallest of its arguments"),
    ("max", "the largest"),
    ("sum", "adds its arguments"),
    ("avg", "the mean of its arguments"),
    ("hypot", "hypot(a, b) — the hypotenuse"),
    ("gcd", "greatest common divisor"),
    ("lcm", "least common multiple"),
];

/// Call `name` with `args`.
pub fn call(name: &str, args: Vec<Value>) -> Result<Value, String> {
    let lower = name.to_ascii_lowercase();
    match lower.as_str() {
        // The ones that keep their unit.
        "abs" => keeping(&lower, args, f64::abs),
        "floor" => keeping(&lower, args, f64::floor),
        "ceil" => keeping(&lower, args, f64::ceil),
        "trunc" => keeping(&lower, args, f64::trunc),
        // **Dimensionless.** `sign` answers which side of zero a value is on, and -1, 0 and 1 are
        // not lengths — `sign(-5 m)` used to answer `-1 m`, which would then add to a length.
        "sign" => sign(args),
        "round" => round(args),
        "sqrt" => root_of(args, 2),
        "cbrt" => root_of(args, 3),
        "root" => {
            let (x, n) = two(&lower, args)?;
            let degree = plain(&lower, &n)?;
            if degree.fract() != 0.0 {
                return Err("a root has to be a whole number of times".to_string());
            }
            root_of(vec![x], degree as i32)
        }
        "min" | "max" | "sum" | "avg" => across(&lower, args),
        // The ones that need a plain number.
        "exp" => plainly(&lower, args, f64::exp),
        "ln" => plainly(&lower, args, f64::ln),
        "log2" => plainly(&lower, args, f64::log2),
        "sin" => plainly(&lower, args, f64::sin),
        "cos" => plainly(&lower, args, f64::cos),
        "tan" => plainly(&lower, args, f64::tan),
        "asin" => plainly(&lower, args, f64::asin),
        "acos" => plainly(&lower, args, f64::acos),
        "atan" => plainly(&lower, args, f64::atan),
        "sinh" => plainly(&lower, args, f64::sinh),
        "cosh" => plainly(&lower, args, f64::cosh),
        "tanh" => plainly(&lower, args, f64::tanh),
        "log" => {
            if args.len() == 1 {
                return plainly(&lower, args, f64::log10);
            }
            let (x, base) = two(&lower, args)?;
            Ok(Value::number(plain(&lower, &x)?.log(plain(&lower, &base)?)))
        }
        "atan2" => {
            let (y, x) = two(&lower, args)?;
            // Both sides may carry a unit as long as it is the *same* one: the ratio is what the
            // function is about, and `atan2(3 m, 4 m)` is a perfectly good question.
            if y.dimension != x.dimension {
                return Err(format!(
                    "atan2 needs two of the same kind, got {} and {}",
                    y.kind(),
                    x.kind()
                ));
            }
            Ok(Value::number(y.number.atan2(x.number)))
        }
        "hypot" => {
            let (a, b) = two(&lower, args)?;
            if a.dimension != b.dimension {
                return Err(format!(
                    "hypot needs two of the same kind, got {} and {}",
                    a.kind(),
                    b.kind()
                ));
            }
            Ok(Value::quantity(a.number.hypot(b.number), a.dimension))
        }
        "gcd" | "lcm" => whole_pair(&lower, args),
        _ => Err(format!("{name:?} is not a function this knows")),
    }
}

fn one(name: &str, mut args: Vec<Value>) -> Result<Value, String> {
    if args.len() != 1 {
        return Err(format!("{name} takes one argument, got {}", args.len()));
    }
    Ok(args.remove(0))
}

fn two(name: &str, mut args: Vec<Value>) -> Result<(Value, Value), String> {
    if args.len() != 2 {
        return Err(format!("{name} takes two arguments, got {}", args.len()));
    }
    let second = args.remove(1);
    Ok((args.remove(0), second))
}

/// A value that must be a plain number, or a diagnostic naming what it was instead.
fn plain(name: &str, value: &Value) -> Result<f64, String> {
    if !value.is_number() {
        return Err(format!("{name} needs a plain number, got {}", value.kind()));
    }
    Ok(value.number)
}

/// Apply `f` and keep the dimension: `abs(-5 m)` is `5 m`.
///
/// **In the unit the answer is shown in, not the base unit.** `..value` carries `shown_as` through
/// unchanged, so applying `f` to the base magnitude produced a number that had been floored in
/// metres and was then labelled in feet: `floor(3.7 ft)` answered `3.28083989501 ft`, and
/// `floor(1.7 km)` answered `1.7 km` because 1700 is already whole. Every such answer looks
/// plausible and none of them is right.
fn keeping(name: &str, args: Vec<Value>, f: impl Fn(f64) -> f64) -> Result<Value, String> {
    let value = one(name, args)?;
    // **Not on an offset scale.** `abs` and `sign` ask which side of zero a value is on, and °C and
    // °F have no true zero to be on a side of — `abs(-5 degC)` is a question about a scale where
    // −5 is not "five below nothing". floor and friends are fine there: they ask for a whole
    // number of degrees, which the scale does answer.
    if matches!(name, "abs" | "sign") && value.shown_as.as_ref().is_some_and(|s| s.offset != 0.0) {
        let unit = value
            .shown_as
            .as_ref()
            .map(|s| s.name.as_str())
            .unwrap_or("");
        return Err(format!(
            "{name} has no meaning on {unit}, which has no true zero — convert to an absolute scale first"
        ));
    }
    Ok(rebuilt(&value, f(value.shown_number())))
}

/// `sign(x)` — which side of zero, as a plain number whatever the operand carried.
///
/// Refused on an offset scale for the reason [`keeping`] gives: °C has no true zero for a value to
/// be on a side of.
fn sign(args: Vec<Value>) -> Result<Value, String> {
    let value = one("sign", args)?;
    if let Some(unit) = value.shown_as.as_ref().filter(|s| s.offset != 0.0) {
        return Err(format!(
            "sign has no meaning on {}, which has no true zero — convert to an absolute scale first",
            unit.name
        ));
    }
    // Not `signum`: that answers 1 for a positive zero and -1 for a negative one, so `sign(0 m)`
    // would be 1 where every calculator says 0.
    let shown = value.shown_number();
    Ok(Value::number(match shown {
        n if n > 0.0 => 1.0,
        n if n < 0.0 => -1.0,
        _ => 0.0,
    }))
}

/// A value holding `shown`, expressed in the unit it is shown in.
///
/// The inverse of [`Value::shown_number`]: a display magnitude back into the base one the rest of
/// the crate does arithmetic in, so the dimension check keeps working.
fn rebuilt(value: &Value, shown: f64) -> Value {
    let number = match &value.shown_as {
        Some(unit) => shown * unit.factor + unit.offset,
        None => shown,
    };
    Value {
        number,
        ..value.clone()
    }
}

/// Apply `f` to a plain number.
fn plainly(name: &str, args: Vec<Value>, f: impl Fn(f64) -> f64) -> Result<Value, String> {
    let value = one(name, args)?;
    Ok(Value::number(f(plain(name, &value)?)))
}

/// `round(x)` to a whole number, `round(x, n)` to `n` decimal places.
fn round(mut args: Vec<Value>) -> Result<Value, String> {
    if args.len() == 1 {
        return keeping("round", args, f64::round);
    }
    let places = args.pop().ok_or("round takes one or two arguments")?;
    let value = args.pop().ok_or("round takes one or two arguments")?;
    let places = plain("round", &places)?;
    let scale = 10f64.powi(places as i32);
    // In the shown unit, for the reason given at `keeping`: rounding the base magnitude and then
    // labelling it in another unit made `round(1.5678 km, 2)` answer `1.5678 km` — 1567.8 metres
    // rounded to two places is itself.
    let shown = (value.shown_number() * scale).round() / scale;
    Ok(rebuilt(&value, shown))
}

/// The `n`th root, which is also where a dimension is divided.
fn root_of(args: Vec<Value>, degree: i32) -> Result<Value, String> {
    let value = one("root", args)?;
    if value.number < 0.0 && degree % 2 == 0 {
        return Err("an even root of a negative number is not a real number".to_string());
    }
    let dimension = value.dimension.rooted(degree).ok_or_else(|| {
        format!(
            "the {} root of {} is not a unit that can be written",
            degree,
            value.kind()
        )
    })?;
    let magnitude = if value.number < 0.0 {
        -(-value.number).powf(1.0 / f64::from(degree))
    } else {
        value.number.powf(1.0 / f64::from(degree))
    };
    Ok(Value::quantity(magnitude, dimension))
}

/// `min`, `max`, `sum` and `avg`, which all want their arguments to be the same kind of thing.
fn across(name: &str, args: Vec<Value>) -> Result<Value, String> {
    let Some(first) = args.first().cloned() else {
        return Err(format!("{name} needs at least one argument"));
    };
    for value in &args {
        if value.dimension != first.dimension {
            return Err(format!(
                "{name} needs arguments of one kind, got {} and {}",
                first.kind(),
                value.kind()
            ));
        }
    }
    let numbers: Vec<f64> = args.iter().map(|v| v.number).collect();
    let answer = match name {
        "min" => numbers.iter().copied().fold(f64::INFINITY, f64::min),
        "max" => numbers.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        "sum" => numbers.iter().sum(),
        _ => numbers.iter().sum::<f64>() / numbers.len() as f64,
    };
    Ok(Value {
        number: answer,
        ..first
    })
}

/// `gcd` and `lcm`, which are about whole numbers.
fn whole_pair(name: &str, args: Vec<Value>) -> Result<Value, String> {
    let (a, b) = two(name, args)?;
    let a = plain(name, &a)?;
    let b = plain(name, &b)?;
    if a.fract() != 0.0 || b.fract() != 0.0 {
        return Err(format!("{name} needs whole numbers"));
    }
    let (mut x, mut y) = (a.abs() as u64, b.abs() as u64);
    while y != 0 {
        let t = y;
        y = x % y;
        x = t;
    }
    let divisor = x;
    let answer = match name {
        "gcd" => divisor as f64,
        _ if divisor == 0 => 0.0,
        _ => (a.abs() as u64 / divisor * b.abs() as u64) as f64,
    };
    Ok(Value::number(answer))
}

/// The dimension a function answers with when it has thrown the unit away.
#[allow(dead_code)]
const PLAIN: Dimension = Dimension::NONE;

#[cfg(test)]
#[path = "functions/tests.rs"]
mod tests;

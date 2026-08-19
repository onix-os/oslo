//! Turning a parsed expression into an answer.
//!
//! # What a bare name means, and in what order
//!
//! A name is looked up as a **variable**, then a **constant**, then a **unit**. That order is the
//! rule: a variable somebody just defined shadows everything, because they defined it on purpose,
//! and `c = 3` followed by `c * 2` must be six rather than six hundred million metres per second.
//! Units come last so that `m`, `s`, `g` and `c` are available as units right up until somebody
//! wants the letter for something else.
//!
//! A name that is none of the three is an error naming itself, because the alternative — treating
//! an unknown name as zero, which some calculators do — turns a typo into a wrong answer.

use crate::dimension::Dimension;
use crate::lex::Base;
use crate::parse::{Binary, Expr, Unary};
use crate::units;
use crate::value::{Shown, Value};
use std::collections::HashMap;

/// The names a session has defined, kept between one expression and the next.
#[derive(Default, Clone, Debug)]
pub struct Scope {
    pub names: HashMap<String, Value>,
    /// Whether a name defined here outlives the expression that defined it.
    ///
    /// False for the one-shot [`crate::calculate`], which builds a fresh scope every call: there,
    /// an assignment is a question with no answer, and the honest reply is to say so rather than
    /// to report the value and drop the name.
    pub remembers: bool,
}

impl Scope {
    /// A scope that keeps what it is told — what a session is built on.
    pub fn new() -> Scope {
        Scope {
            names: HashMap::new(),
            remembers: true,
        }
    }

    /// A scope for a single expression, which refuses to be assigned to.
    pub fn forgetful() -> Scope {
        Scope {
            remembers: false,
            ..Scope::new()
        }
    }
}

/// The constants, in base units. `c` is metres per second, `g` is metres per second squared.
///
/// Every one of these is a name somebody might also want as a variable, which is why the lookup
/// order above puts variables first.
pub const CONSTANTS: &[(&str, f64, Dimension)] = &[
    ("pi", std::f64::consts::PI, Dimension::NONE),
    ("π", std::f64::consts::PI, Dimension::NONE),
    ("tau", std::f64::consts::TAU, Dimension::NONE),
    ("e", std::f64::consts::E, Dimension::NONE),
    ("phi", 1.618_033_988_749_895, Dimension::NONE),
    ("inf", f64::INFINITY, Dimension::NONE),
    // Physical constants, SI 2019 exact where they are exact.
    (
        "c_light",
        299_792_458.0,
        Dimension {
            base: [1, 0, -1, 0, 0, 0, 0],
        },
    ),
    (
        "g_earth",
        9.806_65,
        Dimension {
            base: [1, 0, -2, 0, 0, 0, 0],
        },
    ),
    (
        "G",
        6.674_30e-11,
        Dimension {
            base: [3, -1, -2, 0, 0, 0, 0],
        },
    ),
    (
        "h_planck",
        6.626_070_15e-34,
        Dimension {
            base: [2, 1, -1, 0, 0, 0, 0],
        },
    ),
    (
        "k_boltzmann",
        1.380_649e-23,
        Dimension {
            base: [2, 1, -2, 0, -1, 0, 0],
        },
    ),
    (
        "N_avogadro",
        6.022_140_76e23,
        Dimension {
            base: [0, 0, 0, 0, 0, -1, 0],
        },
    ),
    (
        "R_gas",
        8.314_462_618,
        Dimension {
            base: [2, 1, -2, 0, -1, -1, 0],
        },
    ),
    (
        "e_charge",
        1.602_176_634e-19,
        Dimension {
            base: [0, 0, 1, 1, 0, 0, 0],
        },
    ),
    ("m_electron", 9.109_383_701_5e-31, Dimension::MASS),
    ("m_proton", 1.672_621_923_69e-27, Dimension::MASS),
];

/// Evaluate `expr` against `scope`, which an assignment may add to.
pub fn eval(expr: &Expr, scope: &mut Scope) -> Result<Value, String> {
    match expr {
        Expr::Number(value, base) => Ok(Value::in_base(*value, *base)),
        Expr::Name(name) => name_value(name, scope),
        Expr::Assign(name, body) => {
            if !scope.remembers {
                return Err(format!(
                    "nothing here remembers {name} — a session does: oslo.math.session()"
                ));
            }
            let value = eval(body, scope)?;
            scope.names.insert(name.clone(), value.clone());
            Ok(value)
        }
        Expr::Percent(inner) => {
            let value = eval(inner, scope)?;
            if !value.is_number() {
                return Err(format!("{} cannot be a percentage", value.kind()));
            }
            Ok(Value {
                number: value.number / 100.0,
                percent: true,
                ..value
            })
        }
        Expr::Factorial(inner) => factorial(eval(inner, scope)?),
        Expr::Unary(op, inner) => unary(*op, eval(inner, scope)?),
        Expr::Binary(op, left, right) => {
            let a = eval(left, scope)?;
            let b = eval(right, scope)?;
            binary(*op, a, b)
        }
        Expr::Convert(body, target) => convert(eval(body, scope)?, target, scope),
        Expr::Call(name, args) => {
            let mut values = Vec::with_capacity(args.len());
            for arg in args {
                values.push(eval(arg, scope)?);
            }
            crate::functions::call(name, values)
        }
    }
}

/// A bare name: variable, then constant, then unit.
fn name_value(name: &str, scope: &Scope) -> Result<Value, String> {
    if let Some(value) = scope.names.get(name) {
        return Ok(value.clone());
    }
    if let Some((_, number, dimension)) = CONSTANTS.iter().find(|(n, _, _)| *n == name) {
        return Ok(Value::quantity(*number, *dimension));
    }
    if let Some(unit) = units::resolve(name) {
        // A unit on its own is one of it: `km` is 1000 m, which is what makes `2 km` work as a
        // multiplication and `1 in cm` read as a conversion of one metre-less number.
        return Ok(Value {
            number: unit.factor,
            dimension: unit.dimension,
            shown_as: Some(Shown {
                name: name.to_string(),
                factor: unit.factor,
                offset: unit.offset,
            }),
            base: Base::Decimal,
            percent: false,
            bare_unit: true,
        });
    }
    Err(format!(
        "{name:?} is not a unit, a constant or anything defined"
    ))
}

fn unary(op: Unary, value: Value) -> Result<Value, String> {
    match op {
        Unary::Negate => Ok(Value {
            number: -value.number,
            ..value
        }),
        Unary::Not => {
            let n = whole(&value, "invert")?;
            Ok(Value::in_base(!n as f64, value.base))
        }
    }
}

fn binary(op: Binary, a: Value, b: Value) -> Result<Value, String> {
    match op {
        Binary::Add => a.plus(b),
        Binary::Subtract => a.minus(b),
        Binary::Multiply => a.multiply(b),
        Binary::Divide => a.divide(b),
        Binary::Modulo => a.modulo(b),
        Binary::Power => a.power(b),
        Binary::And | Binary::Or | Binary::Xor | Binary::Shl | Binary::Shr => bitwise(op, a, b),
    }
}

/// The bit operations, which are about whole numbers and nothing else.
///
/// A base is carried through, so `0xf0 | 0x0f` answers `0xff` rather than `255` — the answer comes
/// back in the notation the question was asked in.
fn bitwise(op: Binary, a: Value, b: Value) -> Result<Value, String> {
    let verb = match op {
        Binary::And => "and",
        Binary::Or => "or",
        Binary::Xor => "xor",
        Binary::Shl => "shift",
        _ => "shift",
    };
    let left = whole(&a, verb)?;
    let right = whole(&b, verb)?;
    let out = match op {
        Binary::And => left & right,
        Binary::Or => left | right,
        Binary::Xor => left ^ right,
        Binary::Shl => {
            let places = u32::try_from(right).map_err(|_| "cannot shift by that".to_string())?;
            left.checked_shl(places).ok_or("shifted past the end")?
        }
        Binary::Shr => {
            let places = u32::try_from(right).map_err(|_| "cannot shift by that".to_string())?;
            left.checked_shr(places).ok_or("shifted past the end")?
        }
        _ => unreachable!("bitwise is only called for the bit operations"),
    };
    let base = match a.base {
        Base::Decimal => b.base,
        other => other,
    };
    Ok(Value::in_base(out as f64, base))
}

/// A value as a whole number, for the operations that only mean something on one.
fn whole(value: &Value, verb: &str) -> Result<i64, String> {
    if !value.is_number() {
        return Err(format!("cannot {verb} {}", value.kind()));
    }
    if value.number.fract() != 0.0 {
        return Err(format!(
            "cannot {verb} {}, which is not whole",
            value.number
        ));
    }
    if value.number.abs() > 9.007_199_254_740_992e15 {
        return Err("that number is too large to work on bit by bit".to_string());
    }
    Ok(value.number as i64)
}

fn factorial(value: Value) -> Result<Value, String> {
    let n = whole(&value, "take the factorial of")?;
    if n < 0 {
        return Err("a factorial needs a number that is not negative".to_string());
    }
    if n > 170 {
        return Err("that factorial is too large to hold".to_string());
    }
    let mut out = 1.0f64;
    for step in 2..=n {
        out *= step as f64;
    }
    Ok(Value::in_base(out, value.base))
}

/// `expr in unit`, where the right side names a unit or a base rather than being a value.
fn convert(value: Value, target: &Expr, scope: &mut Scope) -> Result<Value, String> {
    // `255 in hex` is a change of notation, not of unit.
    if let Expr::Name(name) = target
        && let Some(base) = base_named(name)
    {
        if !value.is_number() {
            return Err(format!("{} has no {name} notation", value.kind()));
        }
        return Ok(Value { base, ..value });
    }
    // Otherwise the right side is evaluated as a quantity, and what matters is its unit: `1 mile
    // in km` divides by whatever one kilometre is.
    let unit = eval(target, scope)?;
    // A compound target — `m/s`, `km/h`, `kg m/s^2` — has no single row in the table, so its scale
    // is whatever the expression came to. One written as a single name keeps that name; one built
    // out of several is shown in the base units it amounts to, because there is no other honest
    // way to label `kg m/s^2` once it has been multiplied out.
    let shown = match unit.shown_as.clone() {
        Some(shown) => Shown {
            name: shown.name,
            factor: unit.number,
            offset: shown.offset,
        },
        None if !unit.dimension.is_none() => Shown {
            name: crate::format::base_units(unit.dimension),
            factor: unit.number,
            offset: 0.0,
        },
        None => return Err("the right of `in` has to be a unit".to_string()),
    };
    value.convert_to(&shown, unit.dimension)
}

/// The notations `in` can ask for.
pub fn base_named(name: &str) -> Option<Base> {
    match name.to_ascii_lowercase().as_str() {
        "hex" | "hexadecimal" | "base16" => Some(Base::Hex),
        "bin" | "binary" | "base2" => Some(Base::Binary),
        "oct" | "octal" | "base8" => Some(Base::Octal),
        "dec" | "decimal" | "base10" => Some(Base::Decimal),
        _ => None,
    }
}

#[cfg(test)]
#[path = "eval/tests.rs"]
mod tests;

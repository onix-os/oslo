//! Lua's number: integer or float, and how each is written down.
//!
//! Split from [`super`] because rendering a float is a self-contained problem with one right
//! answer — `%.14g`, whatever the rest of the value tree is doing — and because getting it wrong is
//! invisible until two paths in one runtime print the same number differently.

use std::fmt;

/// A Lua number: integer or float, as 5.4 distinguishes them.
#[derive(Debug, Clone, Copy)]
pub enum Number {
    Int(i64),
    Float(f64),
}

impl Number {
    /// The float value, for arithmetic that always produces one (`/`, `^`).
    pub fn as_float(self) -> f64 {
        match self {
            Number::Int(i) => i as f64,
            Number::Float(f) => f,
        }
    }

    /// The integer value, if this number *has* one exactly.
    ///
    /// `2.0` does convert (Lua calls it an integral float); `2.5` does not. This is the rule
    /// behind "number has no integer representation", which is an error in Lua rather than a
    /// silent truncation — losing the fraction quietly is how an index becomes the wrong element.
    pub fn as_int(self) -> Option<i64> {
        match self {
            Number::Int(i) => Some(i),
            Number::Float(f) if f.fract() == 0.0 && f.is_finite() => Some(f as i64),
            Number::Float(_) => None,
        }
    }
}

impl fmt::Display for Number {
    /// Lua's own formatting: integers print bare, floats always show a decimal point.
    ///
    /// `print(3)` is `3` and `print(3.0)` is `3.0`; without the second rule the two subtypes
    /// become indistinguishable in output, which is the first thing anyone notices.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Number::Int(i) => write!(f, "{i}"),
            Number::Float(x) if x.is_infinite() => {
                write!(f, "{}inf", if *x < 0.0 { "-" } else { "" })
            }
            Number::Float(x) if x.is_nan() => write!(f, "nan"),
            Number::Float(x) if x.fract() == 0.0 && x.abs() < 1e15 => write!(f, "{x:.1}"),
            Number::Float(x) => write!(f, "{}", format_float(*x)),
        }
    }
}

/// Lua prints floats with `%.14g`, which is what makes `0.1 + 0.2` show as `0.3`.
///
/// **All of `%.14g`, not half of it.** The fixed-notation branch used to throw away the mantissa it
/// had just computed and fall back to Rust's `{}`, which prints the shortest string that round-trips
/// — seventeen digits for `0.1 + 0.2`. So the same number rendered two ways in one runtime:
/// `tostring(0.1+0.2)` gave `0.3` and `sh.echo(0.1+0.2)` gave `0.30000000000000004`, and a prompt
/// segment holding a computed ratio showed seventeen digits of noise.
///
/// `%g` picks the notation by exponent — fixed while `-4 <= exp < P`, scientific outside — and
/// trims trailing zeros in both. `P` is the *significant* digit count, so the `e` form that feeds
/// it asks for `P - 1` digits after the point; asking for `P` gave fifteen significant digits.
fn format_float(x: f64) -> String {
    /// The precision Lua uses. Named because it decides both the digit count and where the
    /// notation switches, and those two have to be the same number.
    const P: i32 = 14;

    let scientific = format!("{x:.*e}", (P - 1) as usize);
    let Some((mantissa, exponent)) = scientific.split_once('e') else {
        return scientific;
    };
    let exp: i32 = exponent.parse().unwrap_or(0);
    if !(-4..P).contains(&exp) {
        let mantissa = trimmed(mantissa);
        return format!(
            "{mantissa}e{}{:02}",
            if exp < 0 { '-' } else { '+' },
            exp.abs()
        );
    }
    // Fixed notation, with the digits after the point chosen so the total stays at `P` significant.
    let decimals = (P - 1 - exp).max(0) as usize;
    trimmed(&format!("{x:.decimals$}")).to_string()
}

/// Drop the trailing zeros `%g` drops, and the point if nothing is left after it.
fn trimmed(text: &str) -> &str {
    match text.contains('.') {
        true => text.trim_end_matches('0').trim_end_matches('.'),
        // An integer-valued mantissa written without a point has no zeros to drop: trimming `100`
        // would turn it into `1`.
        false => text,
    }
}

/// **The shared formatter is `%.14g`, the same as the runtime's own `tostring`.**
///
/// The fixed-notation branch used to discard the mantissa it had computed and fall back to Rust's
/// `{}`, which prints the shortest round-tripping form — so one runtime rendered one number two
/// ways: `tostring(0.1+0.2)` was `0.3` while `sh.echo(0.1+0.2)` was `0.30000000000000004`, and the
/// same seventeen digits reached `oslo.json.encode`, prompt segment fields and dropdown lines.
///
/// Every expectation here was taken from the VM rather than written by hand; see
/// `tests/lua_tests.rs` for the round trip that keeps the two honest.
#[cfg(test)]
mod float_tests {
    use super::Number;

    #[track_caller]
    fn shows(x: f64, wanted: &str) {
        assert_eq!(Number::Float(x).to_string(), wanted, "for {x:?}");
    }

    #[test]
    fn a_float_prints_the_way_lua_prints_it() {
        // The one everybody tests a calculator with, and the reason `%.14g` is the rule.
        shows(0.1 + 0.2, "0.3");
        shows(-0.1 - 0.2, "-0.3");
        // Fourteen significant digits, not seventeen and not fifteen.
        shows(1.0 / 3.0, "0.33333333333333");
        shows(1.0 / 7.0, "0.14285714285714");
        shows(100.0 / 7.0, "14.285714285714");
        shows(123456789.123456, "123456789.12346");
        // Trailing zeros go, and the point with them if nothing is left.
        shows(1.5, "1.5");
    }

    /// `%g` switches notation by exponent: scientific below -4 and at or above the precision.
    /// The old bounds were `-5..15`, which is neither.
    #[test]
    fn the_notation_switches_where_lua_switches_it() {
        shows(1e-300, "1e-300");
        shows(0.0000001234, "1.234e-07");
        shows(2.0 / 3.0 * 1e-20, "6.6666666666667e-21");
        shows(1e15, "1e+15");
        shows(1e16, "1e+16");
        shows(1e300, "1e+300");
    }

    /// The values that have no digits to render at all.
    #[test]
    fn the_special_ones_still_read() {
        shows(f64::INFINITY, "inf");
        shows(f64::NEG_INFINITY, "-inf");
        shows(f64::NAN, "nan");
        // A whole-valued float keeps its point, as Lua's does.
        shows(2.0, "2.0");
        shows(0.0, "0.0");
    }
}

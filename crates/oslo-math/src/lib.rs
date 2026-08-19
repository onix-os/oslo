//! Calculation with units, for the shell and for Lua.
//!
//! ```text
//! 2 + 2                        4
//! 5 km in miles                3.10685596119 mi
//! 9.8 m/s^2 * 70 kg            686 N
//! 20% of 250                   50
//! 100 + 10%                    110
//! 0xff | 0x0f                  0xff
//! 255 in binary                0b11111111
//! 20 degC in degF              68 °F
//! sqrt(16 m^2)                 4 m
//! r = 3 ; pi * r^2             28.2743338823
//! ```
//!
//! # Why this is oslo's own rather than a dependency
//!
//! The obvious alternative was to vendor an existing calculator crate. The one this borrows its
//! ideas from pulls `reqwest` and `rustls` — an HTTP client and a TLS stack — so that it can look
//! up currency rates. Inside a static shell binary, to work out `2 + 2`, that is the wrong trade
//! twice over: it is several megabytes and a network stack for arithmetic, and a shell that phones
//! out to evaluate an expression is a shell nobody can run in an initramfs.
//!
//! So this has **no dependencies at all**. A table of units, a recursive-descent parser and f64.
//!
//! # The shape of it
//!
//! | module | what it holds |
//! | --- | --- |
//! | [`dimension`] | what a quantity *is*: seven exponents over the SI bases |
//! | [`units`] | every unit it knows, and the SI prefix rule |
//! | [`lex`] | text to tokens, including `0x`/`0b`/`0o` literals |
//! | [`parse`] | the grammar and its precedence |
//! | [`eval`] | names, constants, conversion, bit operations |
//! | [`functions`] | `sin`, `sqrt`, `min`, `gcd` and the rest |
//! | [`mod@format`] | how an answer is written down |

pub mod dimension;
pub mod eval;
pub mod format;
pub mod functions;
pub mod lex;
pub mod parse;
pub mod units;
pub mod value;

pub use eval::Scope;
pub use value::Value;

/// Work out one expression, with no memory of anything before it.
///
/// The answer is already rendered, because that is what both callers want: the shell prints it and
/// Lua receives it as a string beside the number.
///
/// **An assignment is an error here**, not a value. The scope is built fresh on every call, so
/// `x = 5` could only ever report `5` and forget the name — which reads exactly like it worked.
/// Somewhere to put a name is what [`Scope`] and `oslo.math.session()` are for.
pub fn calculate(source: &str) -> Result<Answer, String> {
    let mut scope = Scope::forgetful();
    calculate_in(source, &mut scope)
}

/// The same, against a scope that remembers what has been defined.
pub fn calculate_in(source: &str, scope: &mut Scope) -> Result<Answer, String> {
    let tokens = lex::lex(source)?;
    if tokens.is_empty() {
        return Err("there is nothing to work out".to_string());
    }
    let expr = parse::parse(&tokens)?;
    let value = eval::eval(&expr, scope)?;
    Ok(Answer {
        text: format::show(&value),
        number: value.shown_number(),
        unit: value
            .shown_as
            .as_ref()
            .map(|shown| shown.name.clone())
            .unwrap_or_default(),
        dimension: value.dimension.describe(),
        value,
    })
}

/// What an expression came to: rendered, and in pieces for a caller that wants them.
#[derive(Clone, Debug)]
pub struct Answer {
    /// The whole answer as it should be shown: `3.10685596119 mi`.
    pub text: String,
    /// The magnitude in the unit it is shown in, for arithmetic on the other side.
    pub number: f64,
    /// The unit's name, or empty for a plain number.
    pub unit: String,
    /// What kind of thing it is: `length`, `a number`, `length·time⁻¹`.
    pub dimension: String,
    /// The value itself, for a caller that wants to keep computing.
    pub value: Value,
}

#[cfg(test)]
#[path = "lib/tests.rs"]
mod tests;

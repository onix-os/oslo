//! Writing an answer down.
//!
//! # The number of digits is the whole problem
//!
//! `0.1 + 0.2` is `0.30000000000000004` in binary floating point, and printing that is technically
//! true and practically useless. Printing `0.3` is what every calculator does, and the way they do
//! it is to render at a precision below the noise floor and drop the trailing zeros.
//!
//! Twelve significant figures: enough that `1/3` shows `0.333333333333` and that a real difference
//! at the twelfth digit still shows, and few enough that the accumulated error of a few operations
//! stays hidden. A value that needs more than that is shown in exponent form instead.

use crate::dimension::superscript;
use crate::lex::Base;
use crate::value::Value;

/// How many significant figures an answer is shown to.
pub const FIGURES: usize = 12;

/// Render `value` the way it should be read back.
pub fn show(value: &Value) -> String {
    if value.base != Base::Decimal {
        return in_base(value);
    }
    let number = value.shown_number();
    let mut out = number_text(number);
    if let Some(shown) = &value.shown_as {
        out.push(' ');
        out.push_str(&shown.name);
    } else if !value.dimension.is_none() {
        // No unit was asked for and none was carried, so the answer is in base units and has to
        // say which: `9.8 m·s⁻²` rather than a bare `9.8`.
        out.push(' ');
        out.push_str(&base_units(value.dimension));
    }
    out
}

/// A number, at [`FIGURES`] significant figures, without trailing zeros.
pub fn number_text(number: f64) -> String {
    if number.is_nan() {
        return "not a number".to_string();
    }
    if number.is_infinite() {
        return if number > 0.0 { "∞" } else { "-∞" }.to_string();
    }
    if number == 0.0 {
        return "0".to_string();
    }
    let magnitude = number.abs();
    // Outside this range the digits are all exponent anyway, and a plain rendering would be a
    // screen of zeros.
    if !(1e-6..1e15).contains(&magnitude) {
        let text = format!("{number:.*e}", FIGURES - 1);
        return tidy_exponent(&text);
    }
    let decimals = FIGURES.saturating_sub(magnitude.log10().floor().max(0.0) as usize + 1);
    let text = format!("{number:.*}", decimals.min(FIGURES));
    trim_zeros(&text)
}

/// Drop the zeros a fixed-precision rendering leaves behind: `1.500000` is `1.5`, `2.000000` is
/// `2`.
fn trim_zeros(text: &str) -> String {
    if !text.contains('.') {
        return text.to_string();
    }
    let trimmed = text.trim_end_matches('0');
    trimmed.trim_end_matches('.').to_string()
}

/// The same, for the mantissa of an exponent form: `1.230000e5` is `1.23e5`.
fn tidy_exponent(text: &str) -> String {
    let Some((mantissa, exponent)) = text.split_once('e') else {
        return text.to_string();
    };
    let exponent = exponent.trim_start_matches('+');
    format!("{}e{exponent}", trim_zeros(mantissa))
}

/// A whole number in the notation it was written in.
fn in_base(value: &Value) -> String {
    let number = value.number;
    if number.fract() != 0.0 || number.abs() > 9.007_199_254_740_992e15 {
        // A base is a way of writing a whole number, and this is not one — so it is shown as the
        // number it is rather than rounded into a notation that cannot hold it.
        return number_text(number);
    }
    let magnitude = number.abs() as u64;
    let digits = match value.base {
        Base::Hex => format!("{magnitude:x}"),
        Base::Binary => format!("{magnitude:b}"),
        Base::Octal => format!("{magnitude:o}"),
        Base::Decimal => magnitude.to_string(),
    };
    let sign = if number < 0.0 { "-" } else { "" };
    format!("{sign}{}{digits}", value.base.prefix())
}

/// The base units of a dimension, as `kg·m·s⁻²`.
///
/// **Mass before length**, which is the order the SI itself writes derived units in — the newton
/// is `kg⋅m⋅s⁻²` in every table there is. The exponents are stored length-first because that is
/// the conventional order to *list the dimensions*; the two orders are different conventions for
/// different jobs, and this is the one an answer is read in.
pub fn base_units(dimension: crate::dimension::Dimension) -> String {
    /// `(index into the exponents, symbol)`, in the order an answer prints them.
    const SYMBOLS: [(usize, &str); 7] = [
        (1, "kg"),
        (0, "m"),
        (2, "s"),
        (3, "A"),
        (4, "K"),
        (5, "mol"),
        (6, "cd"),
    ];
    let mut parts = Vec::new();
    for (index, symbol) in SYMBOLS {
        match dimension.base[index] {
            0 => {}
            1 => parts.push(symbol.to_string()),
            n => parts.push(format!("{symbol}{}", superscript(i32::from(n)))),
        }
    }
    parts.join("·")
}

#[cfg(test)]
#[path = "format/tests.rs"]
mod tests;

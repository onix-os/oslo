//! What a quantity *is*, independently of the unit it is written in.
//!
//! # Why exponents and not names
//!
//! `1 N` and `1 kg·m/s²` are the same thing, and a calculator that compares unit *names* cannot
//! know it. So a quantity carries seven small integers — one exponent per SI base dimension — and
//! every question about compatibility is integer equality. `5 m / 2 s` is dimension
//! `length¹·time⁻¹`; so is `9 km/h`; so the two can be added, and `5 m + 2 s` cannot.
//!
//! Seven, because that is how many the SI has. Angle is deliberately **not** one of them: the
//! radian is dimensionless by definition, and making it a base dimension is what stops
//! `sin(90 deg)` from working. Money is not one either — it has no fixed rate against anything, so
//! a calculator that cannot reach the network cannot answer questions about it honestly.
//!
//! # Why integers and not fractions
//!
//! `√(m²/s²)` is `m/s`, so a square root halves every exponent, and halving an odd exponent needs
//! a fraction. The alternative is refusing `sqrt` on anything but a perfect square of a dimension,
//! which is what a shell calculator can live with: `sqrt(16 m^2)` works and `sqrt(2 m)` says why
//! it cannot. Fractional exponents buy `√Hz` and cost a rational in the hot path of every
//! multiply.

/// The seven SI base dimensions, in the order their exponents are stored.
pub const BASE_NAMES: [&str; 7] = [
    "length",
    "mass",
    "time",
    "current",
    "temperature",
    "amount",
    "luminosity",
];

/// A quantity's dimension: one exponent per base.
///
/// `Copy` and seven bytes, because every arithmetic operation makes one and comparing two is the
/// question asked most often in the evaluator.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Dimension {
    /// Exponents over [`BASE_NAMES`], in that order.
    pub base: [i8; 7],
}

impl Dimension {
    /// A pure number: no units at all. `2`, `sin(x)`, a ratio of two lengths.
    pub const NONE: Dimension = Dimension { base: [0; 7] };

    /// The dimension of one base unit, by index into [`BASE_NAMES`].
    pub const fn base_at(index: usize) -> Dimension {
        let mut base = [0i8; 7];
        base[index] = 1;
        Dimension { base }
    }

    pub const LENGTH: Dimension = Dimension::base_at(0);
    pub const MASS: Dimension = Dimension::base_at(1);
    pub const TIME: Dimension = Dimension::base_at(2);
    pub const CURRENT: Dimension = Dimension::base_at(3);
    pub const TEMPERATURE: Dimension = Dimension::base_at(4);
    pub const AMOUNT: Dimension = Dimension::base_at(5);
    pub const LUMINOSITY: Dimension = Dimension::base_at(6);

    /// Whether this is a plain number.
    pub fn is_none(self) -> bool {
        self == Dimension::NONE
    }

    /// Multiplying quantities adds their dimensions.
    pub fn times(self, other: Dimension) -> Option<Dimension> {
        let mut out = Dimension::default();
        for i in 0..7 {
            out.base[i] = self.base[i].checked_add(other.base[i])?;
        }
        Some(out)
    }

    /// Dividing subtracts them.
    pub fn over(self, other: Dimension) -> Option<Dimension> {
        let mut out = Dimension::default();
        for i in 0..7 {
            out.base[i] = self.base[i].checked_sub(other.base[i])?;
        }
        Some(out)
    }

    /// Raising to an integer power multiplies them.
    pub fn powed(self, exponent: i32) -> Option<Dimension> {
        let mut out = Dimension::default();
        for i in 0..7 {
            out.base[i] = i8::try_from(i32::from(self.base[i]).checked_mul(exponent)?).ok()?;
        }
        Some(out)
    }

    /// The `n`th root, or `None` when an exponent will not divide evenly.
    ///
    /// **Refused rather than rounded.** `sqrt(2 m)` has no dimension this can name, and answering
    /// `1.41 m` — or `1.41` with the unit quietly dropped — is a wrong answer wearing a right
    /// one's clothes.
    pub fn rooted(self, degree: i32) -> Option<Dimension> {
        if degree == 0 {
            return None;
        }
        let mut out = Dimension::default();
        for i in 0..7 {
            if i32::from(self.base[i]) % degree != 0 {
                return None;
            }
            out.base[i] = i8::try_from(i32::from(self.base[i]) / degree).ok()?;
        }
        Some(out)
    }

    /// How a dimension reads when there is no unit name for it: `length·time⁻¹`.
    ///
    /// Only ever seen in a diagnostic, which is exactly when it matters: "expected length, got
    /// length·time⁻¹" tells somebody they divided by a duration and forgot.
    pub fn describe(self) -> String {
        let mut parts: Vec<String> = Vec::new();
        for (index, name) in BASE_NAMES.iter().enumerate() {
            match self.base[index] {
                0 => {}
                1 => parts.push((*name).to_string()),
                n => parts.push(format!("{name}{}", superscript(i32::from(n)))),
            }
        }
        if parts.is_empty() {
            return "a number".to_string();
        }
        parts.join("·")
    }
}

/// An exponent as the digits people write above the line.
///
/// `m²` rather than `m^2` because the answer is read far more often than it is typed, and beside a
/// unit the raised form is the one everybody already knows.
pub fn superscript(n: i32) -> String {
    let digits = ['⁰', '¹', '²', '³', '⁴', '⁵', '⁶', '⁷', '⁸', '⁹'];
    let mut out = String::new();
    if n < 0 {
        out.push('⁻');
    }
    let mut left = n.unsigned_abs();
    let mut stack = Vec::new();
    loop {
        stack.push(digits[(left % 10) as usize]);
        left /= 10;
        if left == 0 {
            break;
        }
    }
    out.extend(stack.iter().rev());
    out
}

#[cfg(test)]
#[path = "dimension/tests.rs"]
mod tests;

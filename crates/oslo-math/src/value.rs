//! A number that knows what it is, and arithmetic that keeps it honest.
//!
//! Everything is held in **base units** — metres, kilograms, seconds — whatever it was written in.
//! `5 km` is `5000` with dimension length, and so is `5000 m`, so the two are the same value and
//! comparing them is comparing two floats. What was *written* survives only as a display
//! preference in [`Value::shown_as`], which is what lets an answer come back in kilometres rather
//! than always in metres.
//!
//! # Percent is a kind of number, not a unit
//!
//! `10%` is `0.1`, and every calculator agrees. What they disagree about is `100 + 10%`: read
//! literally it is `100.1`, and read the way people mean it, it is `110`. This keeps a flag on the
//! value saying "this came from a `%` sign", and addition consults it — see [`Value::plus`]. The
//! flag is dropped by every other operation, so `10% * 2` is `0.2` and nothing further is implied.

use crate::dimension::Dimension;
use crate::lex::Base;

/// A quantity: a number in base units, its dimension, and how to show it.
#[derive(Clone, Debug)]
pub struct Value {
    /// The magnitude, in base units.
    pub number: f64,
    pub dimension: Dimension,
    /// The unit an answer should be rendered in, if a particular one is wanted.
    pub shown_as: Option<Shown>,
    /// The base an answer should be rendered in.
    pub base: Base,
    /// Whether this was written with a `%`.
    pub percent: bool,
    /// Whether this is a bare unit name that nothing has been done to yet.
    ///
    /// **Only the offset units need this, and they need it badly.** `20 °C` is `20 × 1 + 273.15`
    /// kelvin, not `20 × 274.15` — attaching a number to a scale that does not share zero with its
    /// base is not a multiplication at all. Knowing that the right-hand side is still a *bare*
    /// unit is what lets [`Value::multiply`] tell `20 degC` from `20 × (something in °C)`.
    pub bare_unit: bool,
}

/// A unit to render an answer in: its name and how many base units one of it is.
#[derive(Clone, Debug)]
pub struct Shown {
    pub name: String,
    pub factor: f64,
    pub offset: f64,
}

impl Value {
    /// A plain number with no dimension.
    pub fn number(n: f64) -> Value {
        Value {
            number: n,
            dimension: Dimension::NONE,
            shown_as: None,
            base: Base::Decimal,
            percent: false,
            bare_unit: false,
        }
    }

    /// A number written in a particular base, which is also how it will be shown.
    pub fn in_base(n: f64, base: Base) -> Value {
        Value {
            base,
            ..Value::number(n)
        }
    }

    /// A quantity with a dimension.
    pub fn quantity(n: f64, dimension: Dimension) -> Value {
        Value {
            number: n,
            dimension,
            shown_as: None,
            base: Base::Decimal,
            percent: false,
            bare_unit: false,
        }
    }

    pub fn is_number(&self) -> bool {
        self.dimension.is_none()
    }

    /// What this reads as in a diagnostic: `a number`, or `length`.
    pub fn kind(&self) -> String {
        self.dimension.describe()
    }

    /// Keep whichever display preference the operands had.
    ///
    /// The left one wins, so `1 km + 500 m` answers in kilometres: the first unit written is the
    /// one the question was asked in.
    fn shown_from(a: &Value, b: &Value) -> (Option<Shown>, Base) {
        let shown = a.shown_as.clone().or_else(|| b.shown_as.clone());
        let base = match a.base {
            Base::Decimal => b.base,
            other => other,
        };
        (shown, base)
    }

    /// `a + b`, with the percent rule.
    ///
    /// **`100 + 10%` is 110.** A bare `10%` is `0.1` and adding it to a hundred would be `100.1`,
    /// which is arithmetically defensible and never what anybody typing it meant. So a percent on
    /// the right of `+` or `-` is read as a proportion *of the left*, which is the reading every
    /// calculator built for people uses.
    pub fn plus(self, other: Value) -> Result<Value, String> {
        if other.percent && !self.percent {
            let scaled = self.number * other.number;
            return Ok(Value {
                number: self.number + scaled,
                ..self
            });
        }
        self.combine(other, "add", |a, b| a + b)
    }

    pub fn minus(self, other: Value) -> Result<Value, String> {
        if other.percent && !self.percent {
            let scaled = self.number * other.number;
            return Ok(Value {
                number: self.number - scaled,
                ..self
            });
        }
        self.combine(other, "subtract", |a, b| a - b)
    }

    /// Add or subtract, which is the one place two dimensions have to agree.
    fn combine(
        self,
        other: Value,
        verb: &str,
        f: impl Fn(f64, f64) -> f64,
    ) -> Result<Value, String> {
        if self.dimension != other.dimension {
            return Err(format!(
                "cannot {verb} {} and {}",
                self.kind(),
                other.kind()
            ));
        }
        let (shown_as, base) = Value::shown_from(&self, &other);
        Ok(Value {
            number: f(self.number, other.number),
            dimension: self.dimension,
            shown_as,
            base,
            percent: self.percent && other.percent,
            bare_unit: false,
        })
    }

    pub fn multiply(self, other: Value) -> Result<Value, String> {
        // **Attaching a number to an offset unit is not a multiplication.** `20 °C` is
        // `20 × 1 + 273.15` kelvin; multiplied out it would be `20 × 274.15`, which is a
        // temperature on Venus. Only a bare unit on the right qualifies — see `Value::bare_unit`.
        if self.is_number()
            && other.bare_unit
            && let Some(shown) = &other.shown_as
            && shown.offset != 0.0
        {
            return Ok(Value {
                number: self.number * shown.factor + shown.offset,
                dimension: other.dimension,
                shown_as: other.shown_as.clone(),
                base: self.base,
                percent: false,
                bare_unit: false,
            });
        }
        let dimension = self
            .dimension
            .times(other.dimension)
            .ok_or("those units multiply out to something too large to name")?;
        // The display unit only survives when the other side is a plain number: `2 * 3 km` is
        // kilometres, and `3 km * 2 s` is neither of its operands' units.
        let shown_as = match (self.is_number(), other.is_number()) {
            (true, _) => other.shown_as.clone(),
            (_, true) => self.shown_as.clone(),
            _ => None,
        };
        let (_, base) = Value::shown_from(&self, &other);
        Ok(Value {
            number: self.number * other.number,
            dimension,
            shown_as,
            base,
            percent: false,
            bare_unit: false,
        })
    }

    pub fn divide(self, other: Value) -> Result<Value, String> {
        if other.number == 0.0 {
            return Err("divided by zero".to_string());
        }
        let dimension = self
            .dimension
            .over(other.dimension)
            .ok_or("those units divide out to something too large to name")?;
        let shown_as = other.is_number().then(|| self.shown_as.clone()).flatten();
        let (_, base) = Value::shown_from(&self, &other);
        Ok(Value {
            number: self.number / other.number,
            dimension,
            shown_as,
            base,
            percent: false,
            bare_unit: false,
        })
    }

    /// The remainder, which only means something for plain numbers.
    pub fn modulo(self, other: Value) -> Result<Value, String> {
        if other.number == 0.0 {
            return Err("divided by zero".to_string());
        }
        if self.dimension != other.dimension {
            return Err(format!(
                "cannot take {} modulo {}",
                self.kind(),
                other.kind()
            ));
        }
        // **The dimension is kept**, because the remainder of two lengths is a length. Clearing it
        // while `..self` carried the unit *label* through produced `10 m % 3 m` → `1 m` whose
        // dimension said "a number", so `(10 m % 3 m) + 1` answered `2 m` — the unit check that
        // exists to refuse adding a bare number to a length had been disarmed by the operand.
        // The guard above already requires both sides to agree, so there is one dimension to keep.
        Ok(Value {
            number: self.number % other.number,
            ..self
        })
    }

    /// `a ^ b`.
    ///
    /// **A dimension can only be raised to a whole power**, because `m^1.5` is not a unit anybody
    /// can write. A plain number can be raised to anything.
    pub fn power(self, other: Value) -> Result<Value, String> {
        if !other.is_number() {
            return Err(format!("an exponent cannot be {}", other.kind()));
        }
        if self.is_number() {
            return Ok(Value {
                number: self.number.powf(other.number),
                base: self.base,
                ..Value::number(0.0)
            });
        }
        let whole = other.number.round();
        if (other.number - whole).abs() > 1e-9 {
            return Err(format!(
                "{} cannot be raised to a fractional power",
                self.kind()
            ));
        }
        let exponent = whole as i32;
        let dimension = self
            .dimension
            .powed(exponent)
            .ok_or("that power makes a unit too large to name")?;
        Ok(Value::quantity(self.number.powi(exponent), dimension))
    }

    /// Convert into `target`, or say why the two do not measure the same thing.
    pub fn convert_to(self, target: &Shown, dimension: Dimension) -> Result<Value, String> {
        if self.dimension != dimension {
            return Err(format!(
                "cannot convert {} into {}",
                self.kind(),
                dimension.describe()
            ));
        }
        Ok(Value {
            number: self.number,
            dimension: self.dimension,
            shown_as: Some(target.clone()),
            base: self.base,
            percent: false,
            bare_unit: false,
        })
    }

    /// The number as it should be *displayed*, undoing the unit's scale and offset.
    pub fn shown_number(&self) -> f64 {
        match &self.shown_as {
            None => self.number,
            Some(shown) => (self.number - shown.offset) / shown.factor,
        }
    }
}

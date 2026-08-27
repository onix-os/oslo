//! The units this calculator knows, and how a written name becomes one.
//!
//! # One table, and a prefix rule
//!
//! Every unit is `(name, dimension, how many base units one of it is)`. A metre is length and 1;
//! a mile is length and 1609.344; an hour is time and 3600. Conversion is then arithmetic and
//! nothing else: `5 km in miles` is `5 × 1000 ÷ 1609.344`, and the two only combine because both
//! rows carry the same [`Dimension`].
//!
//! SI prefixes are applied by [`resolve`] rather than written out, so the table holds `metre`
//! once and `nanometre`, `centimetre` and `kilometre` come free — as do the ones nobody thought
//! of. A prefix is only tried when the bare name fails, so `min` is a minute rather than a
//! milli-inch.
//!
//! # What is deliberately not here
//!
//! **No currency.** A rate is a fact about today that this program cannot look up, and a
//! calculator that answers a money question with last year's number is worse than one that says it
//! does not know.
//!
//! **The offset units are their own case.** Celsius and Fahrenheit do not scale from kelvin, they
//! *shift* — `0 °C` is 273.15 K, not 0 K — so a plain factor cannot express them and
//! [`Unit::offset`] carries the rest. It is the reason `20 °C + 5 °C` is a question with no good
//! answer and `20 °C + 5 K` has one.

use crate::dimension::Dimension;

/// One row of the table.
#[derive(Clone, Copy, Debug)]
pub struct Unit {
    /// The canonical spelling, used when an answer is printed.
    pub name: &'static str,
    pub dimension: Dimension,
    /// How many base units one of these is. Metre 1, kilometre 1000, mile 1609.344.
    pub factor: f64,
    /// Added *after* scaling, for the units that do not share zero with their base.
    pub offset: f64,
    /// Whether an SI prefix may be written in front of it. False for the imperial units and for
    /// the ones whose name already contains a prefix, so `kilofoot` and `kikilogram` are not words.
    pub prefixable: bool,
}

/// A unit with no prefix, for the table below.
const fn u(name: &'static str, dimension: Dimension, factor: f64) -> Unit {
    Unit {
        name,
        dimension,
        factor,
        offset: 0.0,
        prefixable: false,
    }
}

/// The same, but SI prefixes may be written in front of it.
const fn si(name: &'static str, dimension: Dimension, factor: f64) -> Unit {
    Unit {
        prefixable: true,
        ..u(name, dimension, factor)
    }
}

const NONE: Dimension = Dimension::NONE;
const LENGTH: Dimension = Dimension::LENGTH;
const MASS: Dimension = Dimension::MASS;
const TIME: Dimension = Dimension::TIME;
const CURRENT: Dimension = Dimension::CURRENT;
const TEMP: Dimension = Dimension::TEMPERATURE;
const AMOUNT: Dimension = Dimension::AMOUNT;
const LUMEN: Dimension = Dimension::LUMINOSITY;

/// A dimension built from base exponents, for the derived units.
const fn dim(l: i8, m: i8, t: i8, i: i8, k: i8, n: i8, j: i8) -> Dimension {
    Dimension {
        base: [l, m, t, i, k, n, j],
    }
}

const AREA: Dimension = dim(2, 0, 0, 0, 0, 0, 0);
const VOLUME: Dimension = dim(3, 0, 0, 0, 0, 0, 0);
const SPEED: Dimension = dim(1, 0, -1, 0, 0, 0, 0);
const ACCEL: Dimension = dim(1, 0, -2, 0, 0, 0, 0);
const FORCE: Dimension = dim(1, 1, -2, 0, 0, 0, 0);
const ENERGY: Dimension = dim(2, 1, -2, 0, 0, 0, 0);
const POWER: Dimension = dim(2, 1, -3, 0, 0, 0, 0);
const PRESSURE: Dimension = dim(-1, 1, -2, 0, 0, 0, 0);
const CHARGE: Dimension = dim(0, 0, 1, 1, 0, 0, 0);
const VOLTAGE: Dimension = dim(2, 1, -3, -1, 0, 0, 0);
const RESISTANCE: Dimension = dim(2, 1, -3, -2, 0, 0, 0);
const CAPACITANCE: Dimension = dim(-2, -1, 4, 2, 0, 0, 0);
const FREQUENCY: Dimension = dim(0, 0, -1, 0, 0, 0, 0);
const DENSITY: Dimension = dim(-3, 1, 0, 0, 0, 0, 0);

/// Everything the calculator can name.
///
/// Ordered by subject rather than alphabetically, because that is how somebody scans it looking
/// for whether their unit is here.
pub const UNITS: &[Unit] = &[
    // Length. The metre is the base, so its factor is 1 by definition.
    si("m", LENGTH, 1.0),
    si("metre", LENGTH, 1.0),
    si("meter", LENGTH, 1.0),
    u("in", LENGTH, 0.0254),
    u("inch", LENGTH, 0.0254),
    u("ft", LENGTH, 0.3048),
    u("foot", LENGTH, 0.3048),
    u("feet", LENGTH, 0.3048),
    u("yd", LENGTH, 0.9144),
    u("yard", LENGTH, 0.9144),
    u("mi", LENGTH, 1_609.344),
    u("mile", LENGTH, 1_609.344),
    u("nmi", LENGTH, 1_852.0),
    u("furlong", LENGTH, 201.168),
    u("chain", LENGTH, 20.1168),
    u("fathom", LENGTH, 1.8288),
    u("angstrom", LENGTH, 1e-10),
    u("Å", LENGTH, 1e-10),
    u("au", LENGTH, 1.495_978_707e11),
    u("ly", LENGTH, 9.460_730_472_580_8e15),
    u("lightyear", LENGTH, 9.460_730_472_580_8e15),
    u("pc", LENGTH, 3.085_677_581_491_367e16),
    u("parsec", LENGTH, 3.085_677_581_491_367e16),
    u("thou", LENGTH, 2.54e-5),
    u("mil", LENGTH, 2.54e-5),
    u("pt", LENGTH, 0.000_352_777_777_777_777_8),
    u("pica", LENGTH, 0.004_233_333_333_333_333),
    // Mass. The *gram* is the prefixable name even though the kilogram is the SI base, because
    // `kg` has to spell as a prefix on `g` for `mg` and `µg` to work at all.
    si("g", MASS, 0.001),
    si("gram", MASS, 0.001),
    si("gramme", MASS, 0.001),
    si("t", MASS, 1000.0),
    si("tonne", MASS, 1000.0),
    u("lb", MASS, 0.453_592_37),
    u("lbs", MASS, 0.453_592_37),
    u("pound", MASS, 0.453_592_37),
    u("oz", MASS, 0.028_349_523_125),
    u("ounce", MASS, 0.028_349_523_125),
    u("stone", MASS, 6.350_293_18),
    u("st", MASS, 6.350_293_18),
    u("ton", MASS, 907.184_74),
    u("longton", MASS, 1_016.046_908_8),
    u("carat", MASS, 0.0002),
    u("grain", MASS, 6.479_891e-5),
    u("slug", MASS, 14.593_902_937_206_364),
    u("amu", MASS, 1.660_539_066_60e-27),
    u("dalton", MASS, 1.660_539_066_60e-27),
    // Time. The second is prefixable so `ms`, `µs` and `ns` work; the larger ones are not, or
    // `kilominute` becomes a word.
    si("s", TIME, 1.0),
    si("sec", TIME, 1.0),
    si("second", TIME, 1.0),
    u("min", TIME, 60.0),
    u("minute", TIME, 60.0),
    u("h", TIME, 3_600.0),
    u("hr", TIME, 3_600.0),
    u("hour", TIME, 3_600.0),
    u("day", TIME, 86_400.0),
    u("d", TIME, 86_400.0),
    u("week", TIME, 604_800.0),
    u("wk", TIME, 604_800.0),
    u("fortnight", TIME, 1_209_600.0),
    // The Julian year, which is what an astronomical light-year is defined against.
    u("year", TIME, 31_557_600.0),
    u("yr", TIME, 31_557_600.0),
    u("month", TIME, 2_629_800.0),
    u("decade", TIME, 315_576_000.0),
    u("century", TIME, 3_155_760_000.0),
    // Temperature. Kelvin scales; the other two shift as well — see `Unit::offset`.
    si("K", TEMP, 1.0),
    si("kelvin", TEMP, 1.0),
    Unit {
        name: "°C",
        dimension: TEMP,
        factor: 1.0,
        offset: 273.15,
        prefixable: false,
    },
    Unit {
        name: "degC",
        dimension: TEMP,
        factor: 1.0,
        offset: 273.15,
        prefixable: false,
    },
    Unit {
        name: "celsius",
        dimension: TEMP,
        factor: 1.0,
        offset: 273.15,
        prefixable: false,
    },
    Unit {
        name: "°F",
        dimension: TEMP,
        factor: 5.0 / 9.0,
        offset: 459.67 * 5.0 / 9.0,
        prefixable: false,
    },
    Unit {
        name: "degF",
        dimension: TEMP,
        factor: 5.0 / 9.0,
        offset: 459.67 * 5.0 / 9.0,
        prefixable: false,
    },
    Unit {
        name: "fahrenheit",
        dimension: TEMP,
        factor: 5.0 / 9.0,
        offset: 459.67 * 5.0 / 9.0,
        prefixable: false,
    },
    u("rankine", TEMP, 5.0 / 9.0),
    // Current, amount, luminosity: the remaining three bases.
    si("A", CURRENT, 1.0),
    si("amp", CURRENT, 1.0),
    si("ampere", CURRENT, 1.0),
    si("mol", AMOUNT, 1.0),
    si("mole", AMOUNT, 1.0),
    si("cd", LUMEN, 1.0),
    si("candela", LUMEN, 1.0),
    // Area and volume.
    u("ha", AREA, 10_000.0),
    u("hectare", AREA, 10_000.0),
    u("acre", AREA, 4_046.856_422_4),
    si("L", VOLUME, 0.001),
    si("l", VOLUME, 0.001),
    si("litre", VOLUME, 0.001),
    si("liter", VOLUME, 0.001),
    u("gal", VOLUME, 0.003_785_411_784),
    u("gallon", VOLUME, 0.003_785_411_784),
    u("qt", VOLUME, 0.000_946_352_946),
    u("quart", VOLUME, 0.000_946_352_946),
    u("pint", VOLUME, 0.000_473_176_473),
    u("cup", VOLUME, 0.000_236_588_236_5),
    u("floz", VOLUME, 2.957_352_956_25e-5),
    u("tbsp", VOLUME, 1.478_676_478_125e-5),
    u("tsp", VOLUME, 4.928_921_593_75e-6),
    u("bbl", VOLUME, 0.158_987_294_928),
    // Speed and acceleration.
    u("kph", SPEED, 1000.0 / 3_600.0),
    u("mph", SPEED, 1_609.344 / 3_600.0),
    u("knot", SPEED, 1_852.0 / 3_600.0),
    u("kn", SPEED, 1_852.0 / 3_600.0),
    u("c", SPEED, 299_792_458.0),
    u("gravity", ACCEL, 9.806_65),
    // Force, energy, power, pressure — the derived SI names, which is what makes `1 N` and
    // `1 kg m/s^2` the same quantity rather than two.
    si("N", FORCE, 1.0),
    si("newton", FORCE, 1.0),
    u("lbf", FORCE, 4.448_221_615_260_5),
    u("dyn", FORCE, 1e-5),
    si("J", ENERGY, 1.0),
    si("joule", ENERGY, 1.0),
    si("cal", ENERGY, 4.184),
    si("calorie", ENERGY, 4.184),
    si("Wh", ENERGY, 3_600.0),
    si("eV", ENERGY, 1.602_176_634e-19),
    u("BTU", ENERGY, 1_055.055_852_62),
    u("erg", ENERGY, 1e-7),
    si("W", POWER, 1.0),
    si("watt", POWER, 1.0),
    u("hp", POWER, 745.699_871_582_27),
    si("Pa", PRESSURE, 1.0),
    si("pascal", PRESSURE, 1.0),
    si("bar", PRESSURE, 100_000.0),
    u("atm", PRESSURE, 101_325.0),
    u("psi", PRESSURE, 6_894.757_293_168_361),
    u("torr", PRESSURE, 101_325.0 / 760.0),
    u("mmHg", PRESSURE, 133.322_387_415),
    // Electricity.
    si("C", CHARGE, 1.0),
    si("coulomb", CHARGE, 1.0),
    si("V", VOLTAGE, 1.0),
    si("volt", VOLTAGE, 1.0),
    si("ohm", RESISTANCE, 1.0),
    si("Ω", RESISTANCE, 1.0),
    si("F", CAPACITANCE, 1.0),
    si("farad", CAPACITANCE, 1.0),
    si("Hz", FREQUENCY, 1.0),
    si("hertz", FREQUENCY, 1.0),
    u("rpm", FREQUENCY, 1.0 / 60.0),
    // Density, the one compound people write as a name.
    u("gcc", DENSITY, 1000.0),
    // Angle: dimensionless by definition, which is what lets `sin(90 deg)` work.
    u("rad", NONE, 1.0),
    u("radian", NONE, 1.0),
    u("deg", NONE, std::f64::consts::PI / 180.0),
    u("degree", NONE, std::f64::consts::PI / 180.0),
    u("°", NONE, std::f64::consts::PI / 180.0),
    u("grad", NONE, std::f64::consts::PI / 200.0),
    u("turn", NONE, std::f64::consts::TAU),
    u("arcmin", NONE, std::f64::consts::PI / 10_800.0),
    u("arcsec", NONE, std::f64::consts::PI / 648_000.0),
    // Data. Both families, because both are written and they are not the same size — the whole
    // reason a disk says 500 GB and the computer says 465 GiB.
    u("bit", NONE, 1.0),
    u("byte", NONE, 8.0),
    u("B", NONE, 8.0),
    u("kB", NONE, 8e3),
    u("MB", NONE, 8e6),
    u("GB", NONE, 8e9),
    u("TB", NONE, 8e12),
    u("PB", NONE, 8e15),
    u("KiB", NONE, 8.0 * 1024.0),
    u("MiB", NONE, 8.0 * 1_048_576.0),
    u("GiB", NONE, 8.0 * 1_073_741_824.0),
    u("TiB", NONE, 8.0 * 1_099_511_627_776.0),
    u("PiB", NONE, 8.0 * 1_125_899_906_842_624.0),
];

/// The SI prefixes, longest spelling first so `da` is not read as `d` followed by `a`.
pub const PREFIXES: &[(&str, f64)] = &[
    ("quetta", 1e30),
    ("ronna", 1e27),
    ("yotta", 1e24),
    ("zetta", 1e21),
    ("exa", 1e18),
    ("peta", 1e15),
    ("tera", 1e12),
    ("giga", 1e9),
    ("mega", 1e6),
    ("kilo", 1e3),
    ("hecto", 1e2),
    ("deca", 1e1),
    ("deci", 1e-1),
    ("centi", 1e-2),
    ("milli", 1e-3),
    ("micro", 1e-6),
    ("nano", 1e-9),
    ("pico", 1e-12),
    ("femto", 1e-15),
    ("atto", 1e-18),
    ("zepto", 1e-21),
    ("yocto", 1e-24),
    ("ronto", 1e-27),
    ("quecto", 1e-30),
    ("Q", 1e30),
    ("R", 1e27),
    ("Y", 1e24),
    ("Z", 1e21),
    ("E", 1e18),
    ("P", 1e15),
    ("T", 1e12),
    ("G", 1e9),
    ("M", 1e6),
    ("k", 1e3),
    ("h", 1e2),
    ("da", 1e1),
    ("d", 1e-1),
    ("c", 1e-2),
    ("m", 1e-3),
    ("µ", 1e-6),
    ("μ", 1e-6),
    ("u", 1e-6),
    ("n", 1e-9),
    ("p", 1e-12),
    ("f", 1e-15),
    ("a", 1e-18),
    ("z", 1e-21),
    ("y", 1e-24),
    ("r", 1e-27),
    ("q", 1e-30),
];

/// What a written unit name turned out to mean: a scale over some dimension, and an offset.
#[derive(Clone, Copy, Debug)]
pub struct Resolved {
    pub dimension: Dimension,
    pub factor: f64,
    pub offset: f64,
    /// The spelling to print an answer in, prefix included.
    pub name: &'static str,
    /// The prefix's scale, kept apart so a printed answer can say `km` rather than `1000 m`.
    pub prefix: f64,
}

/// Read a unit name, applying an SI prefix if the bare name is not itself a unit.
///
/// **The bare name is tried first, and that ordering is the whole of the rule.** `min` is a
/// minute, not a milli-inch; `cd` is a candela, not a centi-day; `pt` is a point, not a
/// pico-tonne. Every one of those is a real collision, and every one of them resolves correctly
/// only because a name that is in the table is never taken apart.
pub fn resolve(written: &str) -> Option<Resolved> {
    if let Some(found) = exact_or_prefixed(written) {
        return Some(found);
    }
    // **Plurals, because that is how people write.** `5 km in miles` and `2 h in minutes` are the
    // ordinary way to ask, and refusing them for the sake of a table that only holds singulars is
    // a calculator being pedantic at the user's expense. Tried last, so a unit whose own name ends
    // in `s` — `s` itself, `lbs` — is never mistaken for a plural of something else.
    for singular in [written.strip_suffix('s'), written.strip_suffix("es")] {
        let Some(singular) = singular.filter(|s| !s.is_empty()) else {
            continue;
        };
        if let Some(found) = exact_or_prefixed(singular) {
            return Some(found);
        }
    }
    None
}

/// The name exactly, or an SI prefix in front of one.
fn exact_or_prefixed(written: &str) -> Option<Resolved> {
    if let Some(unit) = UNITS.iter().find(|unit| unit.name == written) {
        return Some(Resolved {
            dimension: unit.dimension,
            factor: unit.factor,
            offset: unit.offset,
            name: unit.name,
            prefix: 1.0,
        });
    }
    for (prefix, scale) in PREFIXES {
        let Some(rest) = written.strip_prefix(prefix) else {
            continue;
        };
        if rest.is_empty() {
            continue;
        }
        let Some(unit) = UNITS
            .iter()
            .find(|unit| unit.name == rest && unit.prefixable)
        else {
            continue;
        };
        // An offset unit with a prefix is not a thing anybody means: `milli°C` has no reading, and
        // scaling the offset would invent one.
        if unit.offset != 0.0 {
            continue;
        }
        return Some(Resolved {
            dimension: unit.dimension,
            factor: unit.factor * scale,
            offset: 0.0,
            name: unit.name,
            prefix: *scale,
        });
    }
    None
}

#[cfg(test)]
#[path = "units/tests.rs"]
mod tests;

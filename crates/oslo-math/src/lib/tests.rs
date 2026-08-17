//! What the calculator answers, end to end.

use super::calculate;

/// Every one of these is a question somebody would type. The expected text is the whole answer,
/// so a change in how numbers or units are rendered shows up here rather than in a screenshot.
#[track_caller]
fn answers(source: &str, wanted: &str) {
    match calculate(source) {
        Ok(answer) => assert_eq!(answer.text, wanted, "for {source:?}"),
        Err(e) => panic!("{source:?} failed: {e}"),
    }
}

#[track_caller]
fn refuses(source: &str, because: &str) {
    match calculate(source) {
        Ok(answer) => panic!("{source:?} answered {:?}, expected a refusal", answer.text),
        Err(e) => assert!(
            e.contains(because),
            "{source:?} said {e:?}, expected something about {because:?}"
        ),
    }
}

#[test]
fn arithmetic() {
    answers("2 + 2", "4");
    answers("2 + 3 * 4", "14");
    answers("(2 + 3) * 4", "20");
    answers("10 / 4", "2.5");
    answers("2^10", "1024");
    answers("2^3^2", "512");
    answers("-2^2", "-4");
    answers("7 % 3", "1");
    answers("5!", "120");
    // The one everybody tests a calculator with.
    answers("0.1 + 0.2", "0.3");
}

#[test]
fn units_and_conversion() {
    answers("5 km in miles", "3.10685596119 miles");
    answers("1 mile in m", "1609.344 m");
    answers("100 cm in m", "1 m");
    answers("1 kg in g", "1000 g");
    answers("2 h in minutes", "120 minutes");
    answers("1 GiB in MiB", "1024 MiB");
}

#[test]
fn dimensions_combine() {
    // Force is mass times acceleration, and the answer knows it.
    answers("70 kg * 9.8 m/s^2", "686 kg·m·s⁻²");
    answers("100 m / 9.58 s", "10.4384133612 m·s⁻¹");
    answers("sqrt(16 m^2)", "4 m");
}

#[test]
fn what_it_refuses() {
    refuses("5 m + 2 s", "cannot add");
    refuses("sqrt(2 m)", "not a unit that can be written");
    refuses("sin(2 m)", "plain number");
    refuses("1 / 0", "divided by zero");
    refuses("nosuchname", "not a unit");
}

#[test]
fn percentages() {
    answers("20%", "0.2");
    answers("20% of 250", "50");
    // The reading people mean, rather than the literal one.
    answers("100 + 10%", "110");
    answers("250 - 10%", "225");
}

#[test]
fn number_bases() {
    answers("0xff", "0xff");
    answers("0b1010", "0b1010");
    answers("255 in hex", "0xff");
    answers("255 in binary", "0b11111111");
    answers("0xff in decimal", "255");
    answers("0xf0 | 0x0f", "0xff");
    answers("0b1100 & 0b1010", "0b1000");
    answers("1 << 10", "1024");
    answers("0xff xor 0x0f", "0xf0");
    answers("~0 & 0xff", "0xff");
}

#[test]
fn constants_and_functions() {
    answers("pi", "3.14159265359");
    answers("sin(0)", "0");
    answers("log(1000)", "3");
    answers("min(3, 1, 2)", "1");
    answers("gcd(12, 18)", "6");
    answers("hypot(3, 4)", "5");
    // Degrees work because the radian is dimensionless.
    answers("sin(90 deg)", "1");
}

#[test]
fn temperature_shifts_rather_than_scaling() {
    answers("20 degC in degF", "68 degF");
    answers("0 degC in K", "273.15 K");
    answers("212 degF in degC", "100 degC");
}

/// A scope remembers, which is what makes a session useful rather than a single sum.
#[test]
fn variables_are_remembered() {
    let mut scope = super::Scope::new();
    super::calculate_in("r = 3", &mut scope).expect("assign");
    let area = super::calculate_in("pi * r^2", &mut scope).expect("area");
    assert_eq!(area.text, "28.2743338823");
}

/// A variable shadows a unit, because it was defined on purpose.
#[test]
fn a_variable_wins_over_a_unit() {
    let mut scope = super::Scope::new();
    super::calculate_in("m = 5", &mut scope).expect("assign");
    let out = super::calculate_in("m * 2", &mut scope).expect("use");
    assert_eq!(out.text, "10");
}

/// **`in` is the inch and the keyword, and both have to work.**
#[test]
fn the_inch_and_the_keyword_share_a_spelling() {
    answers("4 in", "4 in");
    answers("3 ft + 4 in", "3.33333333333 ft");
    answers("2 in in cm", "5.08 cm");
    answers("5 km in miles", "3.10685596119 miles");
}

/// **Side by side binds tighter than divide**, or a speed comes out as metre-seconds.
#[test]
fn juxtaposition_binds_tighter_than_division() {
    answers("100 m / 9.58 s", "10.4384133612 m·s⁻¹");
    answers("9.8 m/s^2", "9.8 m·s⁻²");
    answers("100 km/h in m/s", "27.7777777778 m·s⁻¹");
}

/// A unit built out of several names is still a unit to convert into.
#[test]
fn a_compound_target_is_a_unit() {
    answers("1 N in kg m/s^2", "1 kg·m·s⁻²");
    answers("1 kWh in J", "3600000 J");
}

/// Plurals, because that is how the question is asked.
#[test]
fn plurals_resolve() {
    answers("2 h in minutes", "120 minutes");
    answers("1 day in hours", "24 hours");
    answers("3 metres in cm", "300 cm");
}

/// The prefix rule never takes apart a name that is already a unit.
#[test]
fn a_name_in_the_table_is_never_split_into_a_prefix() {
    // `min` is a minute, not a milli-inch; `cd` a candela, not a centi-day; `pt` a point.
    answers("1 min in s", "60 s");
    answers("2 cd", "2 cd");
    // And a prefix really does apply where the name is not itself a unit.
    answers("1 km in m", "1000 m");
    answers("1 ms in s", "0.001 s");
    answers("1 µm in nm", "1000 nm");
}

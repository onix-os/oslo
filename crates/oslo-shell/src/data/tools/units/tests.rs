//! What the rewrite takes, and — mostly — what it leaves alone.

use super::*;

/// A size becomes the byte count the rows carry.
#[cfg(feature = "math")]
#[test]
fn a_size_becomes_bytes() {
    assert_eq!(expand("size > 1GB"), "size > 1000000000");
    assert_eq!(expand("size > 1 GB"), "size > 1000000000");
    // The binary one is a different number and stays one.
    assert_eq!(expand("size > 1GiB"), "size > 1073741824");
    assert_eq!(expand("size < 500MB"), "size < 500000000");
    // Both sides, and a decimal.
    assert_eq!(
        expand("size > 1.5GB and size < 2GB"),
        "size > 1500000000 and size < 2000000000"
    );
}

/// A duration becomes nanoseconds, which is what a `Val::Duration` reaches Lua as.
#[cfg(feature = "math")]
#[test]
fn a_duration_becomes_nanoseconds() {
    assert_eq!(expand("t > 5min"), "t > 300000000000");
    assert_eq!(expand("t > 2h"), "t > 7200000000000");
    assert_eq!(expand("t > 1ms"), "t > 1000000");
}

/// **The filters that already work must go on working**, which is most of the value of the scan
/// being narrow. Every row here is something somebody could have written before today.
#[test]
fn what_is_already_lua_is_untouched() {
    for same in [
        // Scientific notation is a numeral, not a unit — this is the one that would have broken.
        "size > 1e3",
        "size > 1E3",
        "size > 1.5e-3",
        // A hex numeral Lua reads.
        "flags == 0x1f",
        // Plain numbers and names.
        "size > 1000",
        "not is_dir",
        "name:match('%.rs$')",
        // A digit inside a name begins nothing.
        "x1GB > 2",
        "col2 == 1",
        // Inside quotes it is text: a filter comparing strings compares strings.
        "name == '1GB'",
        "name == \"1GB\"",
        "name == '5min' and size > 1",
    ] {
        assert_eq!(expand(same), same, "rewrote {same:?}");
    }
}

/// A literal the calculator does not know is left exactly as it was, so the expression fails the
/// way it always did rather than in some new way.
#[test]
fn an_unknown_unit_is_left_alone() {
    assert_eq!(expand("size > 1QQ"), "size > 1QQ");
    assert_eq!(expand("size > 3zz"), "size > 3zz");
}

/// The scan must not lose or duplicate anything it passes over.
#[test]
fn everything_else_survives_byte_for_byte() {
    for same in [
        "",
        "   ",
        "a and (b or c)",
        "s == '' and t == \"\"",
        "path:find('/', 1, true)",
        "'unterminated",
    ] {
        assert_eq!(expand(same), same, "changed {same:?}");
    }
}

/// **The exponent cases, which is where this went wrong first.**
///
/// Scanning digits and taking whatever letters followed read `1e3` as one-and-`e`, asked the
/// calculator for Euler's number in bytes, and rewrote a working filter to `size > 0.339785228563`.
/// The numeral is measured in full before a letter can be a unit.
#[test]
fn a_numeral_is_measured_before_a_letter_can_be_a_unit() {
    for same in [
        "size > 1e3",
        "size > 1E3",
        "size > 1e+3",
        "size > 1.5e-3",
        "size > 1.5E-30",
        "a > 1e3 and b < 2e4",
    ] {
        assert_eq!(expand(same), same, "rewrote {same:?}");
    }
}

/// **A Lua keyword is not a unit**, and it is the only place a numeral is legally followed by a
/// word — `n > 1 and m` is valid Lua and must stay exactly as written.
#[test]
fn a_keyword_after_a_number_is_not_a_unit() {
    for same in [
        "n > 1 and m",
        "n > 1 or m",
        "1 and not x",
        "x == 1 and y == 2",
    ] {
        assert_eq!(expand(same), same, "rewrote {same:?}");
    }
}

/// A number followed by something that runs on into a name is nobody's unit.
#[test]
fn a_unit_must_not_run_into_a_name() {
    for same in ["size > 1 GB_x", "size > 1GBx2", "v = 2x3"] {
        assert_eq!(expand(same), same, "rewrote {same:?}");
    }
}

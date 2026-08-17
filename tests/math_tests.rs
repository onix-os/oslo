//! The `math` builtin and `oslo.math`, through the real binary.
//!
//! The engine's own cases live in `crates/oslo-math`; these are about the two doors into it — that
//! the shell joins its arguments so `math 5 km in miles` needs no quoting, and that Lua gets the
//! answer in pieces rather than as one string.

#![cfg(feature = "math")]

mod common;

use common::oslo_bin;
use std::process::{Command, Stdio};

/// Run `-c line` and answer with everything it printed.
#[track_caller]
fn shell(line: &str) -> (String, Option<i32>) {
    let dir = tempfile::tempdir().expect("tempdir");
    let out = Command::new(oslo_bin())
        .arg("-c")
        .arg(line)
        .current_dir(dir.path())
        .env("HOME", dir.path())
        .env("PATH", "/usr/bin:/bin")
        .env_remove("ENV")
        .stdin(Stdio::null())
        .output()
        .expect("spawn oslo");
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    (text.trim_end().to_string(), out.status.code())
}

/// **The arguments are joined**, so the shell's own word splitting does not have to be fought.
#[test]
fn the_words_of_an_expression_need_no_quoting() {
    assert_eq!(shell("math 2 + 2").0, "4");
    assert_eq!(shell("math 5 km in miles").0, "3.10685596119 miles");
    assert_eq!(shell("math 3 ft + 4 in").0, "3.33333333333 ft");
    assert_eq!(shell("math 255 in hex").0, "0xff");
    // Quoted is the same question, for the ones the shell would otherwise act on.
    assert_eq!(shell("math '2 * 3'").0, "6");
}

#[test]
fn units_carry_through_the_arithmetic() {
    assert_eq!(shell("math '9.8 m/s^2 * 70 kg'").0, "686 kg·m·s⁻²");
    assert_eq!(shell("math '100 km/h in m/s'").0, "27.7777777778 m·s⁻¹");
    assert_eq!(shell("math 'sqrt(16 m^2)'").0, "4 m");
    assert_eq!(shell("math '20 degC in degF'").0, "68 degF");
}

#[test]
fn the_pieces_can_be_asked_for_separately() {
    assert_eq!(shell("math --value '5 km in miles'").0, "3.10685596119");
    assert_eq!(shell("math --unit '5 km in miles'").0, "miles");
    assert_eq!(shell("math --kind '100 km/h'").0, "length·time⁻¹");
}

/// A failure says why and is worth a non-zero status, so `math … || …` works.
#[test]
fn a_bad_expression_fails_rather_than_printing_something() {
    let (said, code) = shell("math '5 m + 2 s'");
    assert!(said.contains("cannot add length and time"), "{said}");
    assert_eq!(code, Some(1));

    let (_, unknown) = shell("math --nosuchopt 1");
    assert_eq!(unknown, Some(2));

    let (_, nothing) = shell("math");
    assert_eq!(nothing, Some(2));
}

/// Lua gets the answer in pieces, and a failure as `nil, message` rather than a raise.
#[test]
fn lua_sees_the_answer_in_pieces() {
    let dir = tempfile::tempdir().expect("tempdir");
    let script = dir.path().join("m.lua");
    std::fs::write(
        &script,
        r#"
        local a = oslo.math.eval("5 km in miles")
        print(a.text, a.unit, a.kind)
        print(oslo.math.value("1 GiB in MB"))
        print(oslo.math.convert(100, "km/h", "mph"))
        local bad, why = oslo.math.eval("5 m + 2 s")
        print(tostring(bad), why)
        local s = oslo.math.session()
        s:eval("r = 3")
        print(s:value("pi * r^2"))
        "#,
    )
    .expect("write");
    let out = Command::new(oslo_bin())
        .arg(&script)
        .env("HOME", dir.path())
        .env_remove("ENV")
        .stdin(Stdio::null())
        .output()
        .expect("spawn oslo");
    let said = String::from_utf8_lossy(&out.stdout);
    assert!(
        said.contains("3.10685596119 miles\tmiles\tlength"),
        "{said}"
    );
    assert!(said.contains("1073.741824"), "{said}");
    assert!(said.contains("62.137119223733"), "{said}");
    assert!(said.contains("nil\tcannot add length and time"), "{said}");
    // A session remembers what it was told.
    assert!(said.contains("28.274333882308"), "{said}");
}

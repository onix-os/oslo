//! Known answers, and the round trip.
//!
//! The digests are the published test vectors rather than whatever this code produced when it was
//! written — a checksum asserted against itself is a test that can only ever pass.

use super::super::util::probe;
use super::*;

/// Call `name` on `table` with one argument.
fn call(table: &Value, name: &str, argument: Value) -> Value {
    probe::first(&probe::field(table, name), vec![argument])
}

fn as_text(value: &Value) -> String {
    match value {
        Value::Str(s) => s.to_string(),
        other => panic!("not a string: {}", other.type_name()),
    }
}

#[test]
fn sha256_matches_the_published_vectors() {
    let it = hash();
    // RFC 6234 / NIST: the empty string and "abc".
    assert_eq!(
        as_text(&call(&it, "sha256", Value::str(""))),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        as_text(&call(&it, "sha256", Value::str("abc"))),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}

#[test]
fn sha512_matches_the_published_vector() {
    let it = hash();
    assert_eq!(
        as_text(&call(&it, "sha512", Value::str("abc"))),
        "ddaf35a193617abacc417349ae20413112e6fa4e89a97ea20a9eeee64b55d39a\
         2192992a274fc1a836ba3c23a3feebbd454d4423643ce80e2a9ac94fa54ca49f"
    );
}

/// **Bytes, not a rendering of them.** Hashing what `oslo.fs.read` used to answer for a binary file
/// hashed the replacement characters, so the digest matched nothing and said nothing about why.
#[test]
fn a_digest_is_taken_over_the_bytes_themselves() {
    let it = hash();
    let raw: &[u8] = &[0xff, 0xfe, 0x00];
    let over_bytes = as_text(&call(&it, "sha256", Value::bytes(raw)));
    let over_lossy = as_text(&call(
        &it,
        "sha256",
        Value::str(String::from_utf8_lossy(raw)),
    ));
    assert_ne!(
        over_bytes, over_lossy,
        "the lossy rendering hashed the same as the bytes"
    );
    // And it is the digest of exactly those three bytes.
    assert_eq!(over_bytes, hex_of(&Sha256::digest(raw)));
}

#[test]
fn hex_round_trips_and_refuses_what_is_not_hex() {
    assert_eq!(hex_of(&[0x00, 0x0f, 0xff]), "000fff");
    assert_eq!(bytes_of_hex("000fff"), Some(vec![0x00, 0x0f, 0xff]));
    // Case does not matter: a checksum is copied from wherever it was printed.
    assert_eq!(bytes_of_hex("AbCd"), Some(vec![0xab, 0xcd]));
    // An odd count could be padded at either end, and guessing is how a checksum stops matching.
    assert_eq!(bytes_of_hex("abc"), None);
    assert_eq!(bytes_of_hex("zz"), None);
    assert_eq!(bytes_of_hex(""), Some(Vec::new()));
}

#[test]
fn base64_matches_rfc_4648() {
    for (plain, encoded) in [
        ("", ""),
        ("f", "Zg=="),
        ("fo", "Zm8="),
        ("foo", "Zm9v"),
        ("foob", "Zm9vYg=="),
        ("fooba", "Zm9vYmE="),
        ("foobar", "Zm9vYmFy"),
    ] {
        assert_eq!(base64_of(plain.as_bytes()), encoded, "encoding {plain:?}");
        assert_eq!(
            bytes_of_base64(encoded).as_deref(),
            Some(plain.as_bytes()),
            "decoding {encoded:?}"
        );
    }
}

/// Wrapped base64 is the normal kind — PEM wraps at 64 columns, `base64` at 76.
#[test]
fn base64_ignores_the_wrapping_it_arrives_with() {
    assert_eq!(
        bytes_of_base64("Zm9v\nYmFy\n").as_deref(),
        Some(&b"foobar"[..])
    );
    assert_eq!(
        bytes_of_base64("Zm9v YmFy").as_deref(),
        Some(&b"foobar"[..])
    );
    // Padding is accepted and not required.
    assert_eq!(bytes_of_base64("Zg").as_deref(), Some(&b"f"[..]));
    // One leftover sextet is six bits, which is not a byte.
    assert_eq!(bytes_of_base64("Zm9vY"), None);
    assert_eq!(bytes_of_base64("not base64!"), None);
}

/// Every byte survives the trip out and back, which is the only property that matters.
#[test]
fn every_byte_survives_both_codecs() {
    let all: Vec<u8> = (0..=255u8).collect();
    assert_eq!(bytes_of_hex(&hex_of(&all)).as_deref(), Some(&all[..]));
    assert_eq!(bytes_of_base64(&base64_of(&all)).as_deref(), Some(&all[..]));
}

#[test]
fn the_tables_offer_what_they_say() {
    for (built, names) in [
        (hash(), &["sha256", "sha512", "file"][..]),
        (hex(), &["encode", "decode"][..]),
        (base64(), &["encode", "decode"][..]),
    ] {
        for name in names {
            assert!(
                matches!(probe::field(&built, name), Value::Function(_)),
                "no {name}"
            );
        }
    }
}

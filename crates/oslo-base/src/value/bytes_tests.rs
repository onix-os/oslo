//! One string type, two representations: the rule that keeps that true.
//!
//! See [`Value::Bytes`](super::Value::Bytes) for what the two are and why there are two.

use super::*;

/// **The invariant the two variants rest on**: `Value::bytes` never builds a [`Value::Bytes`]
/// for something that is text. Without this, `t["a"]` and `t[<the bytes "a">]` would be
/// different keys and `"a" == "a"` could be false — Lua has one string type and this is what
/// keeps that true through two representations.
#[test]
fn text_is_never_stored_as_bytes() {
    for text in ["", "a", "hello", "héllo", "🙂", "line\nbreak", "\0embedded"] {
        assert!(
            matches!(Value::bytes(text), Value::Str(_)),
            "{text:?} went in as bytes"
        );
        assert!(Value::bytes(text).lua_eq(&Value::str(text)), "{text:?}");
    }
}

/// And what is not text does become one, keeping every byte.
#[test]
fn what_is_not_text_keeps_its_bytes() {
    let raw: &[u8] = &[0x89, b'P', b'N', b'G', 0xff, 0xfe, 0x00, 0x01];
    let value = Value::bytes(raw);
    assert!(matches!(value, Value::Bytes(_)));
    assert_eq!(value.as_bytes(), Some(raw));
    // One Lua type, whichever half it landed in.
    assert_eq!(value.type_name(), "string");
    assert_eq!(Value::str("a").type_name(), "string");
}

/// A byte string and a text string are never the same key, and never equal — which is exactly
/// right, because no `Bytes` can hold the same bytes as any `Str`.
#[test]
fn the_two_halves_never_collide() {
    let raw = Value::bytes([0xff, 0xfe].as_slice());
    let text = Value::str("a");
    assert!(!raw.lua_eq(&text));
    assert_ne!(Key::from_value(&raw), Key::from_value(&text));

    // The same bytes twice are the same key, so a table can be indexed by one.
    let again = Value::bytes([0xff, 0xfe].as_slice());
    assert!(raw.lua_eq(&again));
    assert_eq!(Key::from_value(&raw), Key::from_value(&again));

    let mut t = Table::new();
    t.set(raw, Value::int(1));
    assert!(matches!(t.get(&again), Value::Number(_)));
}

/// It survives a round trip through a table's key list, which is how `pairs` hands keys back.
#[test]
fn a_byte_key_comes_back_as_itself() {
    let raw: &[u8] = &[0xc3, 0x28];
    let mut t = Table::new();
    t.set(Value::bytes(raw), Value::Bool(true));
    let (key, _) = t.pairs().into_iter().next().expect("one entry");
    assert_eq!(key.as_bytes(), Some(raw));
}

/// A string that is not text is not a number either, however it is spelled.
#[test]
fn bytes_do_not_coerce_to_a_number() {
    assert!(Value::bytes([b'1', 0xff].as_slice()).as_number().is_none());
    assert!(Value::str("12").as_number().is_some());
}

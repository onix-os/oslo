use super::*;

#[test]
fn a_control_byte_is_found_where_it_is() {
    let key = Key::from_byte(0x07);
    assert_eq!(key.byte(), 0x07);
    assert_eq!(key.find(b"\x07"), Some(0));
    assert_eq!(key.find(b"ls -la\x07"), Some(6));
    assert_eq!(key.find(b"ls -la"), None);
}

/// The whole point: the shell inside the scratch turned the protocol on, so the chord arrives spelled
/// the other way and the client must still know it.
#[test]
fn the_same_chord_in_kitty_form_is_the_same_chord() {
    let key = Key::from_byte(0x07);
    assert_eq!(key.find(b"\x1b[103;5u"), Some(0));
    assert_eq!(key.find(b"ls\x1b[103;5u"), Some(2));

    // `^\`, which is the default and the one a legacy terminal sends bare.
    let backslash = Key::from_byte(0x1c);
    assert_eq!(backslash.find(b"\x1c"), Some(0));
    assert_eq!(backslash.find(b"\x1b[92;5u"), Some(0));
}

#[test]
fn the_alternate_key_and_event_sections_are_not_the_chord() {
    let key = Key::from_byte(0x07);
    assert_eq!(key.find(b"\x1b[103:71;5u"), Some(0));
    assert_eq!(key.find(b"\x1b[103;5:1u"), Some(0));
}

#[test]
fn another_chord_on_the_same_key_is_not_it() {
    let key = Key::from_byte(0x07);
    // Alt+g, and g with no modifier at all.
    assert_eq!(key.find(b"\x1b[103;3u"), None);
    assert_eq!(key.find(b"\x1b[103u"), None);
    // Ctrl+h.
    assert_eq!(key.find(b"\x1b[104;5u"), None);
}

/// A sequence that merely contains a `u` is not a chord, however much it looks like one from the
/// wrong end.
#[test]
fn other_sequences_are_left_alone() {
    let key = Key::from_byte(0x07);
    assert_eq!(key.find(b"\x1b[103;5H"), None);
    // The `\x07` ending an OSC is the byte itself, and is only ever found here because this side
    // reads what was *typed*: what the scratch prints is copied out without being looked at.
    assert_eq!(key.find(b"\x1b]0;title\x07u"), Some(9));
    assert_eq!(key.find(b"\x1b[2J"), None);
    assert_eq!(key.find(b"\x1b["), None);
}

/// The default is `^\`, which is dtach's and which nothing in common use binds.
#[test]
fn the_default_key_is_ctrl_backslash() {
    assert_eq!(Key::named("ctrl-\\").byte(), 0x1c);
    assert_eq!(Key::named("ctrl-\\").byte(), DEFAULT);
}

/// Any control chord can be chosen, because the whole point of the setting is that the key is
/// swallowed from everything inside and somebody will need theirs back.
#[test]
fn any_control_chord_works() {
    assert_eq!(Key::named("ctrl-x").byte(), 0x18);
    assert_eq!(Key::named("ctrl-a").byte(), 0x01);
    assert_eq!(Key::named("ctrl-]").byte(), 0x1d);
    // Spelling is not a trap: case and surrounding space are ignored.
    assert_eq!(Key::named("  CTRL-X ").byte(), 0x18);
}

/// A name that cannot be one byte falls back rather than binding something surprising.
///
/// `f4` and the arrows arrive as escape *sequences*, and a program inside a scratch can send the same
/// bytes; swallowing them would make the scratch eat output that was never a keystroke.
#[test]
fn a_key_that_is_not_one_byte_falls_back() {
    for name in ["f4", "up", "alt-x", "ctrl-shift-x", ""] {
        assert_eq!(Key::named(name).byte(), DEFAULT, "{name}");
    }
}

#[test]
fn every_control_byte_names_the_key_it_stands_for() {
    for (byte, code) in [
        (0x01, b'a'),
        (0x07, b'g'),
        (0x1a, b'z'),
        (0x1c, b'\\'),
        (0x1d, b']'),
        (0x1e, b'^'),
        (0x1f, b'_'),
        (0x00, b' '),
    ] {
        assert_eq!(
            Key::from_byte(byte).code,
            u32::from(code),
            "byte {byte:#04x}"
        );
    }
}

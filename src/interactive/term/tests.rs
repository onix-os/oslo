//! Reading keys out of bytes a terminal split however it liked.
//!
//! These moved here with the reader when the history finder stopped being the only thing that
//! needed one. The mouse-report cases are the reason the reader exists: see the module note.

use super::*;

#[test]
fn the_arrows_and_the_keys_beside_them() {
    assert_eq!(key(b"\x1b[A"), Key::Up);
    assert_eq!(key(b"\x1b[B"), Key::Down);
    assert_eq!(key(b"\x1b[C"), Key::Right);
    assert_eq!(key(b"\x1b[D"), Key::Left);
    assert_eq!(key(b"\x1b[H"), Key::Home);
    assert_eq!(key(b"\x1b[F"), Key::End);
    assert_eq!(key(b"\x1b[3~"), Key::Delete);
    assert_eq!(key(b"\x1b[5~"), Key::PageUp);
    assert_eq!(key(b"\x1b[6~"), Key::PageDown);
    assert_eq!(key(b"\x1b[Z"), Key::BackTab);
    // The application-cursor spellings a terminal switches to inside a full-screen program.
    assert_eq!(key(b"\x1bOA"), Key::Up);
    assert_eq!(key(b"\x1bOD"), Key::Left);
}

/// The control characters every readline-shaped thing binds, so a widget does not have to.
#[test]
fn the_control_keys() {
    assert_eq!(key(b"\x10"), Key::Up);
    assert_eq!(key(b"\x0e"), Key::Down);
    assert_eq!(key(b"\x02"), Key::Left);
    assert_eq!(key(b"\x06"), Key::Right);
    assert_eq!(key(b"\x01"), Key::Home);
    assert_eq!(key(b"\x05"), Key::End);
    assert_eq!(key(b"\x04"), Key::Delete);
    assert_eq!(key(b"\x15"), Key::Clear);
    assert_eq!(key(b"\t"), Key::ToggleScope);
}

#[test]
fn the_keys_that_leave() {
    assert_eq!(key(b"\x1b"), Key::Cancel);
    // Ctrl-C is its own key: the pager treats an abort differently from an ordinary quit.
    assert_eq!(key(b"\x03"), Key::Abort);
    assert_eq!(key(b"\r"), Key::Accept);
    assert_eq!(key(b"\n"), Key::Accept);
    assert_eq!(key(b"\x7f"), Key::Backspace);
    assert_eq!(key(b"\x08"), Key::Backspace);
}

#[test]
fn text_is_text() {
    assert_eq!(key(b"g"), Key::Char('g'));
    assert_eq!(key(b" "), Key::Char(' '));
    assert_eq!(key("é".as_bytes()), Key::Char('é'));
    assert_eq!(key("→".as_bytes()), Key::Char('→'));
    // A control character nothing binds is not text.
    assert_eq!(key(b"\x1c"), Key::Ignored);
    assert_eq!(key(&[]), Key::Ignored);
}

/// A mouse report is consumed whole and means nothing.
///
/// The bug this prevents: read in eight-byte chunks, a thirteen-byte report is cut in two and the
/// tail arrives looking like typed text. tmux and hexe both have mouse reporting on, so every
/// movement of the pointer typed into whatever was reading.
#[test]
fn a_mouse_report_is_swallowed() {
    let report = b"\x1b[<35;56;12M";
    match parse(report) {
        Parsed::Took(used, Key::Ignored) | Parsed::Discard(used) => {
            assert_eq!(used, report.len(), "the whole report must go")
        }
        Parsed::Took(_, other) => panic!("a mouse report became {other:?}"),
        Parsed::Partial => panic!("a complete report was called partial"),
    }
    // The X10 encoding, whose three trailing bytes are not part of the sequence by any rule.
    match parse(b"\x1b[M\x20\x21\x22") {
        Parsed::Discard(used) => assert_eq!(used, 6),
        other => panic!("x10 mouse: {other:?}"),
    }
}

#[test]
fn bracketed_paste_markers_are_swallowed() {
    for marker in [b"\x1b[200~".as_slice(), b"\x1b[201~".as_slice()] {
        match parse(marker) {
            Parsed::Took(used, Key::Ignored) | Parsed::Discard(used) => {
                assert_eq!(used, marker.len())
            }
            other => panic!("paste marker leaked: {other:?}"),
        }
    }
}

#[test]
fn an_osc_reply_is_swallowed() {
    match parse(b"\x1b]11;rgb:1e1e/1e1e/2e2e\x1b\\") {
        Parsed::Discard(used) => assert_eq!(used, 25),
        other => panic!("osc leaked: {other:?}"),
    }
    match parse(b"\x1b]0;title\x07") {
        Parsed::Discard(used) => assert_eq!(used, 10),
        other => panic!("osc with BEL leaked: {other:?}"),
    }
}

/// A sequence that has not all arrived waits instead of being read as text. This is the half of
/// the fix that matters: the old code could not wait, so it guessed.
#[test]
fn a_split_sequence_waits_for_the_rest() {
    assert!(matches!(parse(b"\x1b"), Parsed::Partial));
    assert!(matches!(parse(b"\x1b["), Parsed::Partial));
    assert!(matches!(parse(b"\x1b[<35;56"), Parsed::Partial));
    assert!(matches!(parse(b"\x1b]11;rgb:1e"), Parsed::Partial));
    assert!(matches!(parse(&"é".as_bytes()[..1]), Parsed::Partial));
}

#[test]
fn text_is_taken_a_character_at_a_time() {
    assert!(matches!(parse(b"abc"), Parsed::Took(1, Key::Char('a'))));
    assert!(matches!(
        parse("éx".as_bytes()),
        Parsed::Took(2, Key::Char('é'))
    ));
}

/// An arrow arriving in the same read as the text before it must not swallow that text.
#[test]
fn a_key_after_text_is_not_lost() {
    let buf = b"ab\x1b[A";
    assert!(matches!(parse(buf), Parsed::Took(1, Key::Char('a'))));
    assert!(matches!(parse(&buf[1..]), Parsed::Took(1, Key::Char('b'))));
    assert!(matches!(parse(&buf[2..]), Parsed::Took(3, Key::Up)));
}

/// Alt chords, which a terminal spells `ESC` then the character.
///
/// Worth pinning because a config may bind any of them and the decoding is easy to lose: an
/// earlier version discarded `ESC x` outright, so `oslo.keys["alt-p"]` could never fire.
#[test]
fn esc_then_a_character_is_an_alt_chord() {
    assert_eq!(key(&[0x1b, b'p']), Key::Alt('p'));
    assert_eq!(key(&[0x1b, b'b']), Key::Alt('b'));
    assert_eq!(key(&[0x1b, 0x7f]), Key::Alt('\x7f'), "M-DEL");
    // And the whole two-byte sequence is consumed, so the character is not also read as text.
    assert_eq!(parse(&[0x1b, b'p']), Parsed::Took(2, Key::Alt('p')));
}

/// Every control chord decodes, because a config may bind any of them. The ones with a shared
/// meaning keep it.
#[test]
fn control_chords_decode_by_letter() {
    assert_eq!(key(&[0x07]), Key::Ctrl('g'));
    assert_eq!(key(&[0x0f]), Key::Ctrl('o'));
    assert_eq!(key(&[0x01]), Key::Home, "C-a keeps its shared meaning");
    assert_eq!(key(&[0x09]), Key::ToggleScope, "Tab is not C-i here");
}

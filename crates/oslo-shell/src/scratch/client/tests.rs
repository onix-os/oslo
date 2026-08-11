use super::*;

/// The detach byte is the one this side keeps. Everything before it in the same read is still
/// forwarded, so a paste that happens to contain it does not lose the text in front.
///
/// Asserted against the split rather than against a live terminal: `attach` needs a real tty, and
/// what is worth pinning is the rule, not the plumbing around it. It calls the matcher `pump` calls
/// — an earlier version of this helper reimplemented the search, and agreed with itself while the
/// real one was missing every chord a Kitty-mode terminal sent.
fn split(input: &[u8], detach: u8) -> (Vec<u8>, bool) {
    match detach::Key::from_byte(detach).find(input) {
        Some(at) => (input[..at].to_vec(), true),
        None => (input.to_vec(), false),
    }
}

#[test]
fn the_detach_byte_is_the_default_dtach_one() {
    // ^\ is 0x1c. ^X would be 0x18, and is nano's Exit and emacs' prefix.
    assert_eq!(detach::DEFAULT, 0x1c);
    assert_eq!(detach::DEFAULT, b'\\' & 0x1f);
}

#[test]
fn ordinary_keys_are_forwarded_whole() {
    let (forwarded, detached) = split(b"ls -la\r", detach::DEFAULT);
    assert_eq!(forwarded, b"ls -la\r");
    assert!(!detached);
}

/// `^C` is not special here — it is a byte, and the pty at the other end decides it means SIGINT.
#[test]
fn control_c_is_just_a_byte_to_this_side() {
    let (forwarded, detached) = split(b"\x03", detach::DEFAULT);
    assert_eq!(forwarded, b"\x03", "forwarded, not acted on");
    assert!(!detached);
}

#[test]
fn the_detach_byte_ends_the_session() {
    let (forwarded, detached) = split(b"\x1c", detach::DEFAULT);
    assert!(detached);
    assert!(forwarded.is_empty());
}

/// And so does the same chord spelled the other way, which is what a terminal sends once the shell
/// inside the scratch has asked for the Kitty keyboard protocol — as oslo's line editor does.
#[test]
fn the_kitty_spelling_of_the_key_ends_it_too() {
    let (forwarded, detached) = split(b"\x1b[92;5u", detach::DEFAULT);
    assert!(detached);
    assert!(forwarded.is_empty());
}

/// A burst that ends with the key still delivers what came before it.
#[test]
fn what_was_typed_before_the_key_is_not_lost() {
    let (forwarded, detached) = split(b"echo hi\r\x1c", detach::DEFAULT);
    assert_eq!(forwarded, b"echo hi\r");
    assert!(detached);
}

/// Configurable, because somebody who needs `^\` for gdb will want it elsewhere.
#[test]
fn another_key_can_be_chosen() {
    let (forwarded, detached) = split(b"ab\x18cd", 0x18);
    assert_eq!(forwarded, b"ab");
    assert!(detached);
    // And then the default is no longer special.
    let (forwarded, detached) = split(b"a\x1cb", 0x18);
    assert_eq!(forwarded, b"a\x1cb");
    assert!(!detached);
}

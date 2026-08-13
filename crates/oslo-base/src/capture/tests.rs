//! What can be tested without a directory. Keeping and reading back are exercised through the real
//! binary in `tests/capture_tests.rs`: they answer from `$XDG_DATA_HOME`, and a test that pointed
//! the *process* at a temporary one would be changing the environment under every other test's
//! thread — the hazard `track::universal` documents.

use super::*;

/// The end of a long output is the part worth having: the error it stopped at.
#[test]
fn a_trim_keeps_the_end() {
    let text = format!("{}THE-ERROR", "x".repeat(MAX));
    let (kept, trimmed) = trim(&text);
    assert!(trimmed, "it did not report the trim");
    assert!(kept.ends_with("THE-ERROR"), "the end was thrown away");
    assert!(kept.len() <= MAX);
}

/// A trim that fell inside a character would leave a file that is not text at all.
#[test]
fn a_trim_lands_on_a_character_boundary() {
    let text = "é".repeat(MAX);
    let (kept, trimmed) = trim(&text);
    assert!(trimmed);
    assert!(kept.starts_with('é'), "cut through a character");
    assert!(kept.len() <= MAX);
}

/// What fits is kept exactly — trailing blank lines and all, since these are the bytes a clipboard
/// will get.
#[test]
fn what_fits_is_untouched() {
    assert_eq!(trim("trailing\n\n"), ("trailing\n\n", false));
    let edge = "x".repeat(MAX);
    assert_eq!(trim(&edge), (edge.as_str(), false), "MAX itself fits");
}

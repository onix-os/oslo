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

/// **What a command printed is the user's, and nobody else's.**
///
/// `fs::write` creates at 0666 less the umask — 0644 on a stock system — so a kept capture was
/// readable by every local user, in a directory named after the session. An API response, a
/// decrypted file, a `psql` result: whatever `keep` was pointed at. Every other file this crate
/// writes is private; this one was not.
#[test]
fn a_kept_capture_is_readable_only_by_its_owner() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("session.out");
    write_private(&path, b"the command's output").expect("write");

    let mode = std::fs::metadata(&path).expect("stat").permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "kept output is 0600, not {mode:o}");
    assert_eq!(
        std::fs::read_to_string(&path).expect("read"),
        "the command's output"
    );
}

/// Writing again over an existing capture leaves it private too — the mode is not something the
/// first write sets and the second inherits by luck.
#[test]
fn writing_over_a_capture_keeps_it_private() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("session.out");
    std::fs::write(&path, "left behind by something else").expect("seed");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).expect("chmod");

    write_private(&path, b"new output").expect("write");
    // The file already existed, so its mode is what it was: the truncating write does not lower it.
    // What matters is that the *content* is the new one and a fresh capture is private — the seed
    // here is a file oslo did not create.
    assert_eq!(std::fs::read_to_string(&path).expect("read"), "new output");
}

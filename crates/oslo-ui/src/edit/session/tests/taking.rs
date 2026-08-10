//! Accepting a proposal into the line: the ghost suggestion, and the correction.
//!
//! Split from the keymap tests because these are about what a key *takes*, not about what it
//! moves — and because the two proposals meet on the same key and have to be told apart.

use super::*;

/// **Right takes the correction, replacing the line rather than extending it.**
///
/// The one behavioural difference from accepting a suggestion, and the reason the two cannot share
/// a path: a suggestion is appended, a repair is what the line should have been instead.
#[test]
fn right_takes_the_repair_when_there_is_no_suggestion() {
    let mut a = Canned {
        repair: Some("lsblk".into()),
        ..Canned::default()
    };
    let mut s = Session {
        vi: None,
        ..Session::new("lsvlk", 5)
    };
    s.apply(Key::Right, &mut a);
    assert_eq!(s.buffer.text(), "lsblk");
    assert_eq!(s.buffer.cursor(), 5, "the cursor ends after the correction");
}

/// A suggestion wins the key when there is one: it continues what was typed, which is the weaker
/// claim of the two and so the safer thing for a key to do.
#[test]
fn a_suggestion_is_preferred_to_a_repair() {
    let mut a = Canned {
        hint: Some(" -la".into()),
        repair: Some("lsblk".into()),
        ..Canned::default()
    };
    let mut s = Session {
        vi: None,
        ..Session::new("ls", 2)
    };
    s.apply(Key::Right, &mut a);
    assert_eq!(s.buffer.text(), "ls -la");
}

/// Mid-line, Right is a cursor move and nothing else — a correction must not fire from a cursor
/// that is not at the end, exactly as a suggestion does not.
#[test]
fn right_mid_line_still_only_moves() {
    let mut a = Canned {
        repair: Some("lsblk".into()),
        ..Canned::default()
    };
    let mut s = Session {
        vi: None,
        ..Session::new("lsvlk", 2)
    };
    s.apply(Key::Right, &mut a);
    assert_eq!(s.buffer.text(), "lsvlk");
    assert_eq!(s.buffer.cursor(), 3);
}

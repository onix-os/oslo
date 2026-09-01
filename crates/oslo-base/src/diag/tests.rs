use super::*;

fn words(list: &[&str]) -> Snapshot {
    Snapshot::of(list)
}

/// The words become one line, and each of them knows where it landed in it.
#[test]
fn a_snapshot_is_the_words_rejoined() {
    let snap = words(&["kill", "-s", "NOPE", "1"]);
    assert_eq!(snap.text, "kill -s NOPE 1");
    assert_eq!(snap.spans[0], 0..4);
    assert_eq!(snap.spans[2], 8..12);
    assert_eq!(&snap.text[snap.spans[2].clone()], "NOPE");
    assert_eq!(snap.len(), 4);
}

#[test]
fn an_empty_command_is_an_empty_snapshot() {
    let snap: Snapshot = Snapshot::of::<&str>(&[]);
    assert!(snap.is_empty());
    assert_eq!(snap.index_of("anything"), None);
}

/// The word the message names is the word the caret goes under.
#[test]
fn a_word_is_found_by_its_text() {
    let snap = words(&["cols", "name", "nmae"]);
    assert_eq!(snap.index_of("nmae"), Some(2));
    assert_eq!(snap.index_of("name"), Some(1));
    assert_eq!(snap.index_of_positional(0), Some(0));
    assert_eq!(snap.index_of_positional(9), None);
}

/// The **first** match: a builtin complaining about `foo` in `cmd foo foo` looked at the first one.
#[test]
fn a_repeated_word_is_the_first_one() {
    let snap = words(&["cmd", "foo", "foo"]);
    assert_eq!(snap.index_of("foo"), Some(1));
}

/// Answering `None` is ordinary, not a failure: the word in the message may have been rewritten on
/// the way there — a signal upper-cased, a path made absolute — and a caret under the wrong word
/// is worse than none.
#[test]
fn a_word_that_was_rewritten_is_simply_not_found() {
    let snap = words(&["kill", "-s", "term", "1"]);
    assert_eq!(snap.index_of("TERM"), None);
}

/// **The arithmetic that must never panic.** `panic = "abort"` in release means a diagnostic that
/// panics kills the shell while it is already reporting an error.
#[test]
fn a_byte_offset_is_floored_to_a_character() {
    // `é` is two bytes, `字` three.
    let text = "aé字b";
    assert_eq!(floor_boundary(text, 0), 0);
    assert_eq!(floor_boundary(text, 1), 1);
    assert_eq!(floor_boundary(text, 2), 1, "inside é");
    assert_eq!(floor_boundary(text, 3), 3, "after é");
    assert_eq!(floor_boundary(text, 4), 3, "inside 字");
    assert_eq!(floor_boundary(text, 5), 3, "still inside 字");
    assert_eq!(floor_boundary(text, 6), 6, "after 字");
    assert_eq!(floor_boundary(text, 999), text.len(), "past the end");
    // Every answer is a boundary, which is the property that matters.
    for at in 0..=20 {
        assert!(text.is_char_boundary(floor_boundary(text, at)), "{at}");
    }
}

#[test]
fn flooring_an_empty_string_is_zero() {
    assert_eq!(floor_boundary("", 0), 0);
    assert_eq!(floor_boundary("", 7), 0);
}

/// A caret inside a word, for `cols a,b,nmae`.
#[test]
fn a_caret_can_go_inside_one_word() {
    let snap = words(&["cols", "a,b,nmae"]);
    let word = snap.spans[1].clone();
    // `nmae` starts four characters into the operand.
    assert_eq!(&snap.text[word.start + 4..word.start + 8], "nmae");
}

/// Every path out of `draw` on a build that is not drawing answers `false`, which is what tells the
/// caller to print its one-liner. Under `cargo test` stderr is not a terminal, so this is also the
/// live check that a test suite never sees a report.
#[test]
fn nothing_is_drawn_without_a_terminal() {
    let snap = words(&["kill", "-s", "NOPE", "1"]);
    let report = Report {
        message: "oslo: kill: NOPE: invalid signal specification",
        source: "kill",
        label: "not a signal",
        help: None,
    };
    assert!(!enabled(), "a test binary's stderr is not a terminal");
    assert!(!snap.draw(2, &report));
    assert!(!snap.draw_within(2, 0..2, &report));
    assert!(!draw_source("anything", 0..1, &report));
}

/// A word that is not there draws nothing rather than pointing somewhere arbitrary.
#[test]
fn a_caret_under_no_word_draws_nothing() {
    let snap = words(&["cmd"]);
    let report = Report {
        message: "m",
        source: "cmd",
        label: "l",
        help: None,
    };
    assert!(!snap.draw(9, &report));
}

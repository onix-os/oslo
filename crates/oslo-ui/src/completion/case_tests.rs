use super::matches_prefix;
use super::{Match, matchers};
use crate::matching::Fuzzy;

/// Each way of matching, and the case each one exists for.
#[test]
fn a_matcher_answers_what_it_is_for() {
    assert!(Match::Exact.matches("README.md", "REA"));
    assert!(
        !Match::Exact.matches("README.md", "rea"),
        "exact means exact"
    );

    assert!(Match::Ignoring.matches("README.md", "rea"));
    assert!(!Match::Ignoring.matches("README.md", "xyz"));

    // The case everyone describes as "zsh just knew".
    assert!(Match::Pieces.matches("/usr/share/bin", "/u/s/b"));
    assert!(Match::Pieces.matches("foo-bar-baz", "f-b"));
    assert!(Match::Pieces.matches("some_long_name", "s_l_n"));
    assert!(!Match::Pieces.matches("/usr/share/bin", "/u/z/b"));
}

/// Piece matching only applies when a separator was typed. Without that guard it is a plain
/// prefix test wearing another name, tried a second time for nothing.
#[test]
fn piece_matching_needs_a_separator() {
    assert!(
        !Match::Pieces.matches("foo-bar", "foo"),
        "no separator typed"
    );
    assert!(Match::Pieces.matches("foo-bar", "foo-b"));
}

/// More pieces than the candidate has cannot match, whatever they say.
#[test]
fn more_pieces_than_the_candidate_has_never_matches() {
    assert!(!Match::Pieces.matches("/usr/bin", "/u/s/b/x"));
}

/// Exactness first. A list mixing an exact hit with looser ones is worse than either alone.
#[test]
fn the_chain_tries_exactness_first() {
    let chain = matchers(Fuzzy::Off);
    assert_eq!(chain[0], Match::Exact);
    assert_eq!(chain[1], Match::Ignoring);
    assert_eq!(chain[2], Match::Pieces);
}

/// Fuzzy is last, and absent entirely when it is off.
///
/// The order is the whole safety argument for turning it on: it runs only once every stricter
/// pass has come back empty, so a candidate you actually prefixed can never be pushed down the
/// list by one you merely scattered the letters of.
#[test]
fn fuzzy_is_the_last_resort_and_only_when_asked_for() {
    assert_eq!(matchers(Fuzzy::Off).len(), 3, "off adds no pass");
    let chain = matchers(Fuzzy::Smart);
    assert_eq!(chain.len(), 4);
    assert_eq!(*chain.last().unwrap(), Match::Fuzzy(Fuzzy::Smart));
}

/// The setting was read from the config and then ignored, so turning it on changed nothing.
#[test]
fn case_sensitivity_actually_decides_the_match() {
    assert!(matches_prefix("README.md", "RE", false));
    assert!(
        matches_prefix("README.md", "re", false),
        "off means insensitive"
    );
    assert!(!matches_prefix("README.md", "xy", false));

    assert!(matches_prefix("README.md", "RE", true));
    assert!(
        !matches_prefix("README.md", "re", true),
        "turning it on must matter"
    );
}

/// The typed text running past the candidate is not a prefix of it.
#[test]
fn a_longer_typed_word_is_not_a_prefix() {
    assert!(!matches_prefix("ls", "lsof", false));
    assert!(matches_prefix("lsof", "ls", false));
    // An empty prefix matches everything, which is what bare Tab does.
    assert!(matches_prefix("anything", "", false));
}

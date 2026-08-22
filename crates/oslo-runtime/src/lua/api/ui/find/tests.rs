//! The scorer, exercised through the same `Fuzzy` presets a config names.

use super::*;

/// The canonical shell abbreviation, which is what `smart` is tuned for.
#[test]
fn a_shell_abbreviation_reaches_its_command() {
    assert!(fuzzy_score("git checkout", "gco", Fuzzy::Smart).is_some());
}

#[test]
fn a_non_match_scores_nothing_rather_than_zero() {
    assert!(fuzzy_score("git checkout", "zzz", Fuzzy::Smart).is_none());
}

/// Ranking is the whole point: a near-prefix must beat a scatter, or `limit` cuts the wrong ones.
#[test]
fn the_closer_candidate_scores_higher() {
    let close = fuzzy_score("cargo", "ca", Fuzzy::Smart).expect("cargo");
    let far = fuzzy_score("libcargo", "ca", Fuzzy::Smart).expect("libcargo");
    assert!(close > far, "cargo {close} did not beat libcargo {far}");
}

#[test]
fn every_preset_name_a_config_can_write_parses() {
    for name in ["off", "tight", "smart", "loose"] {
        assert!(Fuzzy::parse(name).is_some(), "{name}");
    }
    assert!(Fuzzy::parse("fuzzy").is_none());
}

/// `positions` counts from zero and Lua counts from one; the binding adds the one.
#[test]
fn the_offsets_are_where_the_letters_are() {
    assert_eq!(positions("echo", "ec"), vec![0, 1]);
    assert!(positions("echo", "").is_empty());
}

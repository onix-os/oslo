//! What can be tested without a store of its own. Keeping and reading back go through the real
//! binary in `tests/secrets_tests.rs`: they answer from `$XDG_DATA_HOME`, and a test that pointed
//! the *process* at a temporary one would be changing the environment under every other test's
//! thread — the hazard `track::universal` documents.

use super::*;

/// **A name is a filename**, and one that reaches out of the directory is refused rather than
/// quietly rewritten: a secret stored somewhere other than where it was asked for is worse than an
/// error, because nothing looks wrong until the day it is read by whoever owns that other place.
#[test]
fn a_name_cannot_reach_out_of_the_store() {
    for bad in ["..", "../elsewhere", "a/b", "", ".hidden", "with\0nul"] {
        assert!(path(bad).is_err(), "{bad:?} was accepted");
    }
}

/// An ordinary name lands in the store, under its own name, with the format's extension.
#[test]
fn a_name_is_a_file_in_the_store() {
    let Ok(path) = path("deploy-token") else {
        return; // No $HOME in this environment; nothing to assert about where it would go.
    };
    assert!(path.ends_with("deploy-token.age"), "{}", path.display());
    assert!(path.to_string_lossy().contains("oslo/secrets"));
}

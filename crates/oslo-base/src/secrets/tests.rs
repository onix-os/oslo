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

/// **A directory called `.git` is not a repository**, and the difference is the whole value of the
/// warning: the machine this was written on has an empty `~/.git` that `git` itself does not
/// recognise, and the first version of the check told its owner their key was about to be
/// committed to it.
#[test]
fn an_empty_dot_git_is_not_a_repository() {
    let dir = tempfile::tempdir().expect("tempdir");
    let empty = dir.path().join("looks-like-one");
    std::fs::create_dir_all(&empty).expect("an empty .git");
    assert!(!is_repository(&empty));

    // A real one has `HEAD` in it…
    std::fs::write(empty.join("HEAD"), "ref: refs/heads/main\n").expect("write");
    assert!(is_repository(&empty));

    // …and a worktree or submodule has a *file* saying where its directory is.
    let pointer = dir.path().join("worktree-dot-git");
    std::fs::write(&pointer, "gitdir: /elsewhere\n").expect("write");
    assert!(is_repository(&pointer));
}

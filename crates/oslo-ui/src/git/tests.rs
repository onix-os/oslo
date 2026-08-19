//! Repositories built by hand, because what is under test is the *reading* of `.git`.
//!
//! Nothing here runs `git`. A test that did would be testing git, and would also be asserting that
//! whatever git is installed lays its directory out the way this expects — which is the thing worth
//! knowing and the thing such a test would hide.

use super::*;

/// Sha-1-shaped object ids, so `is_object_id` accepts them.
const A: &str = "1111111111111111111111111111111111111111";
const B: &str = "2222222222222222222222222222222222222222";

/// Write a file, making its parents.
fn put(root: &Path, at: &str, body: &str) {
    let path = root.join(at);
    std::fs::create_dir_all(path.parent().expect("a parent")).expect("mkdir");
    std::fs::write(path, body).expect("write");
}

#[test]
fn an_object_id_is_forty_or_sixty_four_hex_digits() {
    assert!(is_object_id(A));
    assert!(is_object_id(&"a".repeat(64)));
    assert!(!is_object_id("ref: refs/heads/main"));
    assert!(!is_object_id(&"z".repeat(40)));
    assert!(!is_object_id(""));
}

/// **The loose ref wins.** Both exist after `git gc` and a commit, and `packed-refs` is then the
/// older of the two — reading it first reports a commit the branch has already left.
#[test]
fn a_loose_ref_is_preferred_to_the_packed_one() {
    let git = tempfile::tempdir().expect("temp");
    put(
        git.path(),
        "packed-refs",
        &format!("# pack-refs\n{B} refs/heads/main\n"),
    );
    assert_eq!(resolve(git.path(), "refs/heads/main").as_deref(), Some(B));

    put(git.path(), "refs/heads/main", &format!("{A}\n"));
    assert_eq!(resolve(git.path(), "refs/heads/main").as_deref(), Some(A));
}

/// A `^`-prefixed line in `packed-refs` is the peeled target of the tag above it, not a ref.
#[test]
fn a_peeled_line_is_not_mistaken_for_a_ref() {
    let git = tempfile::tempdir().expect("temp");
    put(
        git.path(),
        "packed-refs",
        &format!("# pack-refs with: peeled\n{A} refs/tags/v1\n^{B}\n"),
    );
    assert_eq!(resolve(git.path(), "refs/tags/v1").as_deref(), Some(A));
}

#[test]
fn a_config_section_is_read_by_its_quoted_name() {
    let config = "[core]\n\trepositoryformatversion = 0\n\
                  [branch \"develop\"]\n\tremote = origin\n\tmerge = refs/heads/develop\n\
                  [branch \"main\"]\n\tremote = upstream\n";
    let section = section_of(config, "branch \"develop\"").expect("the section");
    assert_eq!(setting(&section, "remote").as_deref(), Some("origin"));
    assert_eq!(
        setting(&section, "merge").as_deref(),
        Some("refs/heads/develop")
    );
    // And it stops at the next header rather than running into it.
    assert_eq!(setting(&section, "repositoryformatversion"), None);
    assert!(section_of(config, "branch \"nothing\"").is_none());
}

#[test]
fn a_missing_setting_is_none_rather_than_empty() {
    let section = section_of("[a]\nx = 1\n", "a").expect("the section");
    assert_eq!(setting(&section, "y"), None);
}

/// **A linked worktree's own directory holds almost nothing.** `HEAD` is there; the refs are in the
/// repository it was linked from, and `commondir` is how git says where that is. Reading a branch's
/// commit without following it finds nothing, which showed up as `head().commit == nil` inside every
/// worktree.
#[test]
fn a_ref_is_found_through_commondir() {
    let repo = tempfile::tempdir().expect("temp");
    let main = repo.path().join("main.git");
    let linked = main.join("worktrees/feature");

    put(&main, "refs/heads/feature", &format!("{A}\n"));
    put(&linked, "commondir", "../..\n");
    put(&linked, "HEAD", "ref: refs/heads/feature\n");

    assert_eq!(resolve(&linked, "refs/heads/feature").as_deref(), Some(A));
}

/// A per-worktree ref wins over the shared one, which is what makes `refs/bisect/*` work.
#[test]
fn the_worktrees_own_ref_is_preferred() {
    let repo = tempfile::tempdir().expect("temp");
    let main = repo.path().join("main.git");
    let linked = main.join("worktrees/feature");

    put(&main, "refs/bisect/bad", &format!("{A}\n"));
    put(&linked, "commondir", "../..\n");
    put(&linked, "refs/bisect/bad", &format!("{B}\n"));

    assert_eq!(resolve(&linked, "refs/bisect/bad").as_deref(), Some(B));
}

#[test]
fn a_relative_commondir_is_resolved_without_dot_dots_left_in() {
    let joined = normalise(Path::new("/a/b/.git/worktrees/w/../.."));
    assert_eq!(joined, Path::new("/a/b/.git"));
}

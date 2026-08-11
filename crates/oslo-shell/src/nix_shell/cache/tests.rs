//! **Nothing here moves the process's working directory or its environment.** Both are shared with
//! every other test in the crate, and these two failed in the full suite while passing alone until
//! they took the project root and the cache root as parameters instead.

use super::*;

/// A project with a flake in it, and somewhere to cache answers about it.
fn project() -> (tempfile::TempDir, tempfile::TempDir) {
    let root = tempfile::tempdir().expect("temp dir");
    let base = tempfile::tempdir().expect("temp dir");
    std::fs::write(root.path().join("flake.nix"), "{}").expect("flake");
    (root, base)
}

fn argv(args: &[&str]) -> Vec<String> {
    args.iter().map(|a| (*a).to_string()).collect()
}

/// **An edited flake re-evaluates, and an untouched one does not.**
///
/// The whole value of the cache is that it is keyed rather than timed, and the whole risk is
/// serving a dev shell that no longer matches the flake that describes it.
#[test]
fn the_key_moves_when_an_input_does_and_not_otherwise() {
    let project = tempfile::tempdir().expect("temp dir");
    let root = project.path();
    std::fs::write(root.join("flake.nix"), "{ }").expect("write");

    let first = key(root, &[String::from(".")]);
    assert_eq!(
        first,
        key(root, &[String::from(".")]),
        "nothing changed, nothing to re-do"
    );
    assert_ne!(
        first,
        key(root, &[String::from("..#other")]),
        "another shell, another key"
    );

    std::fs::write(root.join("flake.nix"), "{ inputs = {}; }").expect("write");
    assert_ne!(
        first,
        key(root, &[String::from(".")]),
        "an edited flake must re-evaluate"
    );
}

/// A project with no flake at all still has a stable key rather than a changing one.
#[test]
fn a_project_with_no_inputs_is_still_answerable() {
    let project = tempfile::tempdir().expect("temp dir");
    assert_eq!(
        key(project.path(), &[String::from(".")]),
        key(project.path(), &[String::from(".")])
    );
}

#[test]
fn a_kept_document_comes_back() {
    let (root, base) = project();
    let args = argv(&["flake", "metadata"]);
    assert_eq!(
        document_in(base.path(), root.path(), &args),
        None,
        "nothing kept yet"
    );
    keep_in(base.path(), root.path(), &args, r#"{"description":"x"}"#);
    assert_eq!(
        document_in(base.path(), root.path(), &args).as_deref(),
        Some(r#"{"description":"x"}"#)
    );
}

#[test]
fn editing_the_flake_invalidates_it() {
    let (root, base) = project();
    let args = argv(&["flake", "show"]);
    keep_in(base.path(), root.path(), &args, r#"{"a":1}"#);
    assert!(document_in(base.path(), root.path(), &args).is_some());

    std::fs::write(
        root.path().join("flake.nix"),
        r#"{ description = "moved"; }"#,
    )
    .expect("write");
    assert_eq!(
        document_in(base.path(), root.path(), &args),
        None,
        "a flake that changed must be asked again"
    );
}

#[test]
fn two_queries_do_not_share_an_entry() {
    let (root, base) = project();
    keep_in(
        base.path(),
        root.path(),
        &argv(&["flake", "show"]),
        r#"{"which":"show"}"#,
    );
    keep_in(
        base.path(),
        root.path(),
        &argv(&["flake", "metadata"]),
        r#"{"which":"metadata"}"#,
    );
    assert_eq!(
        document_in(base.path(), root.path(), &argv(&["flake", "show"])).as_deref(),
        Some(r#"{"which":"show"}"#)
    );
    assert_eq!(
        document_in(base.path(), root.path(), &argv(&["flake", "metadata"])).as_deref(),
        Some(r#"{"which":"metadata"}"#)
    );
}

#[test]
fn the_same_query_from_two_projects_is_two_entries() {
    let (root, base) = project();
    let (elsewhere, _) = project();
    let args = argv(&["flake", "metadata"]);
    assert_ne!(
        document_path(base.path(), root.path(), &args),
        document_path(base.path(), elsewhere.path(), &args),
        "one project's answer would serve the other"
    );
}

#[test]
fn a_document_is_readable_only_by_its_owner() {
    // It can hold whatever a flake evaluates to, and the umask would leave it world-readable.
    let (root, base) = project();
    let args = argv(&["print-dev-env"]);
    keep_in(base.path(), root.path(), &args, r#"{"token":"hunter2"}"#);
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(document_path(base.path(), root.path(), &args))
        .expect("stat")
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600, "{mode:o}");
}

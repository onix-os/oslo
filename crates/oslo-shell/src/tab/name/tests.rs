use super::*;

/// A name is a filename, so it is refused rather than rewritten.
#[test]
fn a_name_that_could_escape_the_directory_is_refused() {
    assert!(valid("alpha"));
    assert!(valid("build-2"));
    assert!(valid("A_b-9"));

    assert!(!valid(""), "empty");
    assert!(!valid("../etc/passwd"), "traversal");
    assert!(!valid("a/b"), "separator");
    assert!(!valid(".hidden"), "leading dot");
    assert!(!valid("a b"), "space");
    assert!(!valid("a\nb"), "newline");
    assert!(!valid(&"x".repeat(65)), "too long");
}

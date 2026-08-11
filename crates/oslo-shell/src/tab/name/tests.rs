use super::*;

#[test]
fn the_first_suggestion_is_alpha() {
    assert_eq!(suggest::<String>(&[]), "alpha");
}

#[test]
fn it_skips_what_is_taken() {
    assert_eq!(suggest(&["alpha", "beta"]), "gamma");
    // Order of what is taken does not matter; the answer is the first *free* one.
    assert_eq!(suggest(&["beta", "alpha"]), "gamma");
    // A gap is filled before moving on.
    assert_eq!(suggest(&["alpha", "gamma"]), "beta");
}

/// A name of your own does not consume a Greek letter.
#[test]
fn an_unrelated_name_changes_nothing() {
    assert_eq!(suggest(&["build", "deploy"]), "alpha");
}

/// Past the alphabet it still answers, because refusing would be worse than being ugly.
#[test]
fn it_still_answers_when_the_alphabet_runs_out() {
    let all: Vec<String> = GREEK.iter().map(|s| s.to_string()).collect();
    assert_eq!(suggest(&all), "tab-1");

    let mut more = all.clone();
    more.push("tab-1".to_string());
    assert_eq!(suggest(&more), "tab-2");
}

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

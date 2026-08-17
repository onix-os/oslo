//! The edge cases the string surgery this replaces used to get wrong.

use super::*;

fn env_with(name: &str, value: &str) -> Environment {
    let mut env = Environment::new();
    env.set_var(name, value, true);
    env
}

#[test]
fn empty_entries_are_never_produced_or_kept() {
    let env = env_with("P", "/a::/b:");
    assert_eq!(entries(&env, "P"), ["/a", "/b"]);

    // A variable that is not set is an empty list, not a list holding "".
    assert!(entries(&env, "NOTHING").is_empty());
}

/// **Reloading a configuration must not grow the variable.** This is the failure that made direnv's
/// `PATH_add` grow `$PATH` to pages, and it is why an entry already present moves rather than
/// repeats.
#[test]
fn prepending_twice_leaves_one_entry_at_the_front() {
    let mut env = env_with("P", "/usr/bin:/bin");
    let base = Path::new("/base");
    prepend(&mut env, "P", &["/opt/tool".to_string()], base);
    assert_eq!(entries(&env, "P"), ["/opt/tool", "/usr/bin", "/bin"]);

    prepend(&mut env, "P", &["/opt/tool".to_string()], base);
    assert_eq!(entries(&env, "P"), ["/opt/tool", "/usr/bin", "/bin"]);

    // And an entry already further down is *moved*, so what was asked for wins.
    prepend(&mut env, "P", &["/bin".to_string()], base);
    assert_eq!(entries(&env, "P"), ["/bin", "/opt/tool", "/usr/bin"]);
}

/// Appending is the other half, and it must not demote what is already preferred.
#[test]
fn appending_leaves_an_existing_entry_where_it_was() {
    let mut env = env_with("P", "/opt/tool:/usr/bin");
    let base = Path::new("/base");
    append(&mut env, "P", &["/fallback".to_string()], base);
    assert_eq!(entries(&env, "P"), ["/opt/tool", "/usr/bin", "/fallback"]);

    // `/opt/tool` was put first on purpose; appending it again must not move it last.
    append(&mut env, "P", &["/opt/tool".to_string()], base);
    assert_eq!(entries(&env, "P"), ["/opt/tool", "/usr/bin", "/fallback"]);
}

/// **`./bin` means the caller's bin**, not the bin of wherever the shell later stands.
#[test]
fn a_relative_entry_is_resolved_where_it_was_written() {
    let mut env = env_with("P", "/bin");
    prepend(&mut env, "P", &["./tools".to_string()], Path::new("/proj"));
    assert_eq!(entries(&env, "P"), ["/proj/tools", "/bin"]);

    // And `..` is taken lexically, without asking the disk about anything.
    assert_eq!(absolute("../x", Path::new("/a/b")), Path::new("/a/x"));
    assert_eq!(absolute("/abs", Path::new("/a/b")), Path::new("/abs"));
}

#[test]
fn removal_takes_a_pattern_and_says_how_many_went() {
    let mut env = env_with("P", "/nix/a:/usr/bin:/nix/b");
    assert_eq!(remove(&mut env, "P", &["/nix/*".to_string()]), 2);
    assert_eq!(entries(&env, "P"), ["/usr/bin"]);
    assert_eq!(remove(&mut env, "P", &["/nothing".to_string()]), 0);
}

#[test]
fn containment_compares_the_way_prepending_writes() {
    let env = env_with("P", "/proj/tools:/bin");
    assert!(contains(&env, "P", "./tools", Path::new("/proj")));
    assert!(contains(&env, "P", "/bin", Path::new("/proj")));
    assert!(!contains(&env, "P", "/sbin", Path::new("/proj")));
}

#[test]
fn the_pattern_matcher_crosses_slashes_and_survives_many_stars() {
    assert!(glob("/nix/*", "/nix/store/abc"));
    assert!(glob("*", ""));
    assert!(glob("a?c", "abc"));
    assert!(!glob("a?c", "ac"));
    assert!(glob("*a*b*c*", "xxaxxbxxcxx"));
    assert!(!glob("*a*b*c*", "xxaxxcxx"));
    // Nothing but stars against a long subject must not take exponential time.
    assert!(glob(&"*".repeat(20), &"a".repeat(200)));
}

/// **Rebuilding the list collapses duplicates that were already in it.** A side effect rather than
/// a goal, and worth pinning either way — a caller counting entries before and after an add would
/// otherwise be surprised by a list that did not grow.
#[test]
fn a_change_collapses_duplicates_that_were_already_there() {
    let mut env = env_with("P", "/usr/bin:/opt:/usr/bin");
    assert_eq!(entries(&env, "P").len(), 3);

    prepend(&mut env, "P", &["/new".to_string()], Path::new("/base"));
    assert_eq!(entries(&env, "P"), ["/new", "/usr/bin", "/opt"]);
}

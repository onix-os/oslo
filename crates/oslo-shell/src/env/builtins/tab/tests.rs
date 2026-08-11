use super::*;

/// The usage text is what `-h` prints and what an unknown flag prints after its complaint, so it
/// has to name every form the builtin actually takes.
#[test]
fn the_usage_names_every_form() {
    for form in ["tab ", "tab <name>", "tab -l"] {
        assert!(USAGE.contains(form), "{form:?} is not in the usage");
    }
}

/// A name is not a flag. `tab -x` is a mistake and `tab my-tab` is not, and answering them the same
/// way would make the common case look like an error.
#[test]
fn a_leading_dash_is_the_only_thing_read_as_a_flag() {
    for flag in ["-x", "--nope", "-"] {
        assert!(flag.starts_with('-'), "{flag:?} must be read as a flag");
    }
    for name in ["work", "api-2", "A_b9", "l"] {
        assert!(!name.starts_with('-'), "{name:?} must be read as a name");
    }
}

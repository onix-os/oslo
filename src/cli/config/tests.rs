use super::*;

#[test]
fn the_help_names_both_subcommands() {
    let plain = help(Paint::plain());
    assert!(!plain.contains('\x1b'), "plain is plain");
    for name in ["files", "which"] {
        assert!(plain.contains(name), "{name} is missing");
    }
    assert!(plain.starts_with("USAGE\n"), "{plain}");
}

#[test]
fn a_subcommand_nobody_has_is_a_usage_error() {
    assert_eq!(run(&["nonesuch".to_string()]), 2);
    // `which` with nothing to look up is one too.
    assert_eq!(run(&["which".to_string()]), 2);
}

#[test]
fn help_is_success_and_no_arguments_is_not() {
    assert_eq!(run(&["--help".to_string()]), 0);
    assert_eq!(run(&[]), 2);
}

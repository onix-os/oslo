use super::*;

#[test]
fn the_help_names_both_subcommands() {
    let plain = MENU.overview(Paint::plain());
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

/// **The same as every other tool's**: a bare tool name is the help page, and the help page is not
/// a failure. `config` used to exit 2 here and nothing else did, which is one rule to remember for
/// one command.
#[test]
fn a_bare_tool_name_is_the_help_page() {
    assert_eq!(run(&["--help".to_string()]), 0);
    assert_eq!(run(&[]), 0);
}

use super::*;

/// The name oslo was called by is what selects a tool — with or without a directory in front, as
/// `$PATH` and a shebang respectively supply it.
#[test]
fn a_tool_is_chosen_by_the_name_it_was_called_by() {
    assert_eq!(from_argv0("oslo-config").map(|t| t.name), Some("config"));
    assert_eq!(
        from_argv0("/usr/bin/oslo-history").map(|t| t.name),
        Some("history")
    );
    assert_eq!(from_argv0("./oslo-hook").map(|t| t.name), Some("hook"));
}

/// **The shell's own names are not tools.** This is the whole safety property: `oslo` and `sh`
/// reach the shell, and no tool name can ever be reached the way a script path is.
#[test]
fn the_shell_is_not_a_tool() {
    for called_as in ["oslo", "sh", "/bin/sh", "/usr/bin/oslo", "rush"] {
        assert!(
            from_argv0(called_as).is_none(),
            "{called_as} must start the shell"
        );
    }
}

/// A login shell arrives as `-sh`, and a `#!/bin/oslo` script arrives with the script in argv[1],
/// never in argv[0]. Neither can be read as a tool.
#[test]
fn a_login_shell_is_not_a_tool() {
    for called_as in ["-sh", "-oslo", "-bash"] {
        assert!(
            from_argv0(called_as).is_none(),
            "{called_as} is a login shell"
        );
    }
}

/// A name oslo does not have is not a tool, rather than a tool that fails later.
#[test]
fn an_unknown_name_is_not_a_tool() {
    assert!(from_argv0("oslo-nonesuch").is_none());
    assert!(from_argv0("oslo-").is_none());
    assert!(from_argv0("config").is_none(), "the prefix is required");
}

/// Every tool has a name that can be a file name and an `about` that can be printed. The table
/// feeds both the dispatcher and the help, so a malformed row breaks both.
#[test]
fn every_tool_is_well_formed() {
    for tool in TOOLS {
        assert!(
            tool.name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c == '-'),
            "{}: a tool name becomes a file name",
            tool.name
        );
        assert!(!tool.about.is_empty(), "{}: needs a description", tool.name);
        assert_eq!(
            from_argv0(&format!("oslo-{}", tool.name)).map(|t| t.name),
            Some(tool.name),
            "{}: listed but not reachable",
            tool.name
        );
    }
}

/// Two tools cannot share a name, or one of them is unreachable.
#[test]
fn tool_names_are_unique() {
    let mut names: Vec<&str> = TOOLS.iter().map(|t| t.name).collect();
    names.sort_unstable();
    let count = names.len();
    names.dedup();
    assert_eq!(names.len(), count, "duplicate tool name");
}

/// A name nothing on `$PATH` answers to is not linked. Asserted with a name no machine has, so
/// this says something whether or not oslo is installed.
#[test]
fn an_uninstalled_tool_is_not_linked() {
    assert!(!linked("definitely-not-a-real-tool-name"));
}

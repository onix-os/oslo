use super::*;

/// A name oslo does not have is not a tool, rather than a tool that fails later.
#[test]
fn an_unknown_name_is_not_a_tool() {
    assert!(from_name("nonesuch").is_none());
    assert!(from_name("").is_none());
}

/// Every tool has a name that can be typed and an `about` that can be printed. The table feeds
/// both the dispatcher and the help, so a malformed row breaks both.
#[test]
fn every_tool_is_well_formed() {
    for tool in TOOLS {
        assert!(
            tool.name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c == '-'),
            "{}: a tool name is typed at a prompt",
            tool.name
        );
        assert!(!tool.about.is_empty(), "{}: needs a description", tool.name);
        assert_eq!(
            from_name(tool.name).map(|t| t.name),
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

/// Nothing of that name on disk.
fn nothing(_: &str) -> bool {
    false
}

/// A bare tool name in the operand slot is the tool — which is what makes `oslo history` work.
#[test]
fn a_bare_tool_name_is_a_tool() {
    assert_eq!(
        as_operand_when("history", nothing).map(|t| t.name),
        Some("history")
    );
    assert_eq!(
        as_operand_when("config", nothing).map(|t| t.name),
        Some("config")
    );
    assert!(as_operand_when("nonesuch", nothing).is_none());
}

/// **A path is never a tool.** Every shebang produces a slashed argv[1] — `./config` when run from
/// the current directory, the full path when found on `$PATH` — so this one condition is what
/// keeps `#!/bin/oslo` scripts working whatever they are named.
#[test]
fn anything_with_a_slash_is_a_path() {
    for path in ["./config", "/usr/bin/history", "bin/hook", "../direnv"] {
        assert!(as_operand_when(path, nothing).is_none(), "{path} is a path");
        // And a slashed path is refused before the filesystem is even consulted, so a shebang
        // works the same whether or not the script happens to exist yet.
        assert!(as_operand_when(path, |_| true).is_none(), "{path}");
    }
}

/// **A real file always wins.** The safety property: `oslo config` beside a `./config` runs the
/// script, so nothing that works today can change meaning. The alternative when no such file
/// exists was never "run something else" — oslo does not search `$PATH` for a script operand — it
/// was `No such file or directory`. Only an error becomes useful.
#[test]
fn an_existing_file_beats_the_tool() {
    for tool in TOOLS {
        assert!(
            as_operand_when(tool.name, |_| true).is_none(),
            "{}: a script of that name must win",
            tool.name
        );
        assert!(
            as_operand_when(tool.name, nothing).is_some(),
            "{}: with no such file it is the tool",
            tool.name
        );
    }
}

/// **A directory of that name does not take the tool away.** The operand slot is a *script* path,
/// and a directory is never a script — so `oslo config` in a project holding a `config/` directory
/// used to answer `No such file or directory` instead of opening the configuration. `make/`,
/// `config/` and `history/` are ordinary names for a directory to have.
#[test]
fn a_directory_does_not_shadow_a_tool() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir(dir.path().join("config")).expect("a config directory");
    std::fs::write(dir.path().join("history"), "#!/bin/sh\n").expect("a history script");

    let is_file = |word: &str| dir.path().join(word).is_file();
    assert_eq!(
        as_operand_when("config", is_file).map(|t| t.name),
        Some("config"),
        "a directory is not a script and must not shadow the tool"
    );
    // …while a real file of that name still wins, which is the rule this exists for.
    assert!(
        as_operand_when("history", is_file).is_none(),
        "a script called `history` is what `oslo history` was asked to run"
    );
}

use super::*;

/// A name that could reach outside the directory is not a spec name.
#[test]
fn a_name_that_could_reach_out_of_the_directory_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("python3.11.yaml"), "name: python3.11\n").unwrap();
    let dirs = [dir.path().to_path_buf()];
    assert!(find_in(&dirs, "").is_none());
    assert!(find_in(&dirs, "../../etc/passwd").is_none());
    assert!(find_in(&dirs, ".ssh/config").is_none());
    // â¦while a dot inside a name is ordinary: plenty of commands have one.
    assert!(find_in(&dirs, "python3.11").is_some());
}

#[test]
fn a_spec_is_found_by_the_name_of_its_file() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("mycmd.yaml"),
        "name: mycmd\ndescription: my command\ncommands:\n  - name: sub\n",
    )
    .unwrap();
    // **The filename decides which command the spec answers for.** A `name:` that disagreed with
    // it would leave the file unreachable — the failure that reads as a bug in the completion
    // rather than a typo in the spec.
    std::fs::write(dir.path().join("real.yml"), "name: something-else\n").unwrap();
    let dirs = [dir.path().to_path_buf()];

    let spec = find_in(&dirs, "mycmd").expect("found");
    assert_eq!(spec.description, "my command");
    assert_eq!(spec.subcommands[0].name, "sub");
    assert_eq!(find_in(&dirs, "real").map(|s| s.name), Some("real".into()));
    assert!(find_in(&dirs, "absent").is_none());
}

/// A file that does not parse is reported and skipped, rather than taking the directory with it.
#[test]
fn a_broken_spec_costs_only_itself() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("broken.yaml"), "name: &anchor\n").unwrap();
    std::fs::write(dir.path().join("fine.yaml"), "name: fine\n").unwrap();
    let dirs = [dir.path().to_path_buf()];
    assert!(find_in(&dirs, "broken").is_none());
    assert!(find_in(&dirs, "fine").is_some());
}

/// **Yours is searched before oslo's, and oslo's is not in a directory you keep things in.**
/// `make configs` mirrors the shipped set with `rsync --delete`; a hand-written spec sharing that
/// root would be deleted by installing the shell.
#[test]
fn your_own_directory_comes_before_the_shipped_one() {
    // SAFETY: single-threaded test, and these are read only by `directories`.
    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", "/x/config");
        std::env::set_var("XDG_DATA_HOME", "/x/data");
    }
    let dirs = directories();
    let yours = dirs.iter().position(|d| d.starts_with("/x/config/oslo"));
    let shipped = dirs.iter().position(|d| d.starts_with("/x/data/oslo"));
    assert!(yours.is_some() && shipped.is_some(), "{dirs:?}");
    assert!(yours < shipped, "yours must win: {dirs:?}");
    unsafe {
        std::env::remove_var("XDG_CONFIG_HOME");
        std::env::remove_var("XDG_DATA_HOME");
    }
}

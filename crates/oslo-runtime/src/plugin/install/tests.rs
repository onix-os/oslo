use super::*;

fn candidate(name: &str, builtins: &str) -> (tempfile::TempDir, PathBuf) {
    let root = tempfile::tempdir().expect("temp dir");
    let directory = root.path().join(name);
    std::fs::create_dir_all(&directory).expect("mkdir");
    std::fs::write(
        directory.join("plugin.lua"),
        format!(r#"return {{ name = {name:?}, builtins = {builtins} }}"#),
    )
    .expect("manifest");
    std::fs::write(directory.join("init.lua"), "-- nothing\n").expect("entry");
    (root, directory)
}

fn installed(name: &str, builtins: &[&str]) -> Installed {
    Installed {
        name: name.to_string(),
        entry: "init.lua".to_string(),
        builtins: builtins.iter().map(|n| (*n).to_string()).collect(),
        tools: Vec::new(),
        hash: "x".to_string(),
        requires: None,
        load_on: None,
    }
}

#[test]
fn a_local_directory_is_a_source_and_a_missing_one_is_not() {
    let (_root, directory) = candidate("notes", r#"{ "note" }"#);
    assert_eq!(
        Source::parse(directory.to_str().unwrap()).expect("parse"),
        Source::Path(directory.clone())
    );
    assert!(Source::parse("/nonexistent/plugin").is_err());
}

/// **A revision is required**, because the trust hash makes a moving target unusable.
#[test]
fn a_git_source_must_name_a_revision() {
    assert_eq!(
        Source::parse("github:user/repo@v1.0").expect("parse"),
        Source::Git {
            url: "https://github.com/user/repo".to_string(),
            revision: "v1.0".to_string(),
        }
    );
    assert_eq!(
        Source::parse("https://git.test/u/r@abc123").expect("parse"),
        Source::Git {
            url: "https://git.test/u/r".to_string(),
            revision: "abc123".to_string(),
        }
    );
    let refused = Source::parse("github:user/repo").expect_err("should refuse");
    assert!(refused.contains("name a revision"), "{refused}");
}

#[test]
fn planning_reads_the_manifest_and_hashes_without_running_anything() {
    let (_root, directory) = candidate("notes", r#"{ "note" }"#);
    let planned = plan(&directory, &[]).expect("plan");
    assert_eq!(planned.manifest.name, "notes");
    assert!(!planned.hash.is_empty());
    assert!(planned.conflicts.is_empty());
}

/// A name another plugin already reserves is reported before anything is written.
#[test]
fn a_name_another_plugin_holds_is_a_conflict() {
    let (_root, directory) = candidate("notes", r#"{ "note", "n" }"#);
    let planned = plan(&directory, &[installed("other", &["note"])]).expect("plan");
    assert_eq!(planned.conflicts, ["note"]);

    // Reinstalling *itself* is not a conflict with itself.
    let planned = plan(&directory, &[installed("notes", &["note"])]).expect("plan");
    assert!(planned.conflicts.is_empty());
}

#[test]
fn copying_leaves_out_the_repository_and_every_symlink() {
    let (_root, directory) = candidate("notes", r#"{ "note" }"#);
    std::fs::create_dir_all(directory.join(".git")).expect("mkdir");
    std::fs::write(directory.join(".git/HEAD"), "ref: refs/heads/main").expect("write");
    std::fs::create_dir_all(directory.join("lib")).expect("mkdir");
    std::fs::write(directory.join("lib/helper.lua"), "-- helper").expect("write");
    std::os::unix::fs::symlink("/etc/hostname", directory.join("linked.lua")).expect("symlink");

    let into = tempfile::tempdir().expect("temp dir");
    let destination = into.path().join("notes");
    copy_tree(&directory, &destination).expect("copy");

    assert!(destination.join("plugin.lua").is_file());
    assert!(destination.join("lib/helper.lua").is_file(), "nested Lua");
    assert!(
        !destination.join(".git").exists(),
        "the repository came too"
    );
    assert!(
        !destination.join("linked.lua").exists(),
        "a symlink came too"
    );
}

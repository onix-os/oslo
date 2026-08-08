use super::*;

#[test]
fn directories_sort_first_and_hidden_entries_are_optional() {
    let dir = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir(dir.path().join("z-dir")).expect("mkdir");
    std::fs::write(dir.path().join("a-file"), "x").expect("file");
    std::fs::write(dir.path().join(".hidden"), "x").expect("hidden");

    let (visible, error) = read(dir.path(), false);
    assert!(error.is_none());
    assert_eq!(visible.len(), 2);
    assert_eq!(visible[0].name, "z-dir");
    assert!(visible[0].directory);

    let (all, _) = read(dir.path(), true);
    assert_eq!(all.len(), 3);
    assert!(all.iter().any(|entry| entry.name == ".hidden"));
}

#[test]
fn filtering_ranks_names() {
    let entries = vec![
        Entry {
            name: "cargo.toml".to_string(),
            directory: false,
            symlink: false,
            size: 0,
            mode: 0,
            modified: SystemTime::UNIX_EPOCH,
        },
        Entry {
            name: "src".to_string(),
            directory: true,
            symlink: false,
            size: 0,
            mode: 0,
            modified: SystemTime::UNIX_EPOCH,
        },
    ];
    assert_eq!(narrow(&entries, "src", Fuzzy::Smart), vec![1]);
    assert_eq!(narrow(&entries, "", Fuzzy::Smart), vec![0, 1]);
}

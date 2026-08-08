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

/// **An empty directory still has a path to show.**
///
/// The width is taken from the widest row, and a directory with no rows has none — so the box
/// collapsed to a single column and the heading rendered as one `…`, which is what the whole
/// widget became the moment you walked into an empty directory. The path is part of the block and
/// has to count towards its width like anything else.
#[test]
fn a_directory_with_nothing_in_it_is_still_as_wide_as_its_path() {
    let look = crate::ui::ask::look::Preset::History.look();
    let gutter = look.pad + crate::ui::prompt::printed_width(&look.marker);
    let view = View {
        selected: 0,
        offset: 0,
        height: 0,
        query: "",
        matched: 0,
        total: 0,
        marked: 0,
        cols: 0,
        filtering: false,
        elapsed_ms: 0,
    };

    let empty = look.natural_width(&[], &view);
    assert_eq!(empty, 0, "nothing to measure");

    let path = "/tmp/somewhere/with/a/reasonably/long/name";
    let needed = empty.max(crate::ui::prompt::printed_width(path) + gutter);
    assert!(
        needed >= path.len(),
        "the box must not be narrower than the path it is showing: {needed}"
    );
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

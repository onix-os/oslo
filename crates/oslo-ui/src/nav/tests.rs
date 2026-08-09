use super::*;
use std::time::SystemTime;

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
    let look = crate::ask::look::Preset::History.look();
    let gutter = look.pad + crate::prompt::printed_width(&look.marker);
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
    let needed = empty.max(crate::prompt::printed_width(path) + gutter);
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
            modified: SystemTime::UNIX_EPOCH,
        },
        Entry {
            name: "src".to_string(),
            directory: true,
            symlink: false,
            size: 0,
            modified: SystemTime::UNIX_EPOCH,
        },
    ];
    assert_eq!(narrow(&entries, "src", Fuzzy::Smart), vec![1]);
    assert_eq!(narrow(&entries, "", Fuzzy::Smart), vec![0, 1]);
}

/// The mark is chosen by extension when the configuration names one, and by kind otherwise.
#[test]
fn an_extension_chooses_the_mark_and_a_directory_always_wins() {
    let icons = crate::settings::Icons {
        directory: "■".to_string(),
        file: "≡".to_string(),
        by_extension: vec![("rs".to_string(), "R".to_string())],
    };
    assert_eq!(icons.of("main.rs", false), "R");
    // Matched without regard to case, because `README.MD` is the same kind of file as `readme.md`.
    assert_eq!(icons.of("MAIN.RS", false), "R");
    assert_eq!(icons.of("Makefile", false), "≡");
    // A name that merely begins with a dot has no extension — `.gitignore` is not a `gitignore`.
    assert_eq!(icons.of(".gitignore", false), "≡");
    // A directory called `src.rs` is still a directory.
    assert_eq!(icons.of("src.rs", true), "■");
}

/// A `Navigator` pointed at `at`, with type-and-navigate configured.
fn walker(at: &std::path::Path, enabled: bool, settle_ms: u64) -> Navigator {
    Navigator {
        start: at.to_path_buf(),
        hidden: false,
        width: 0,
        height: 0,
        fuzzy: Fuzzy::Smart,
        icons: crate::settings::Icons::default(),
        type_nav: crate::settings::TypeNav {
            enabled,
            settle: std::time::Duration::from_millis(settle_ms),
        },
        chrome: crate::ask::chrome::Chrome::default(),
        look: crate::ask::Preset::History.look(),
    }
}

/// Type one letter at a time, as the widget does, and answer where it ended up.
fn type_into(spec: &Navigator, word: &str) -> State {
    let mut state = State::new(spec);
    for c in word.chars() {
        if state.still_settling(spec) {
            continue;
        }
        state.query.push(c);
        state.refilter(spec.fuzzy);
        state.walk_into_the_only_match(spec);
    }
    state
}

/// The whole point: `fuzz` stops being ambiguous at `fuz`, so the walk happens with a `z` still to
/// come — and that `z` must not become a filter in the directory just entered.
#[test]
fn the_tail_of_the_word_that_walked_you_in_is_not_a_search() {
    let root = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir(root.path().join("fuzz")).expect("mkdir");
    // Shares `fu`, so the name is only unambiguous at `fuz` — one character short of the word.
    std::fs::create_dir(root.path().join("fudge")).expect("mkdir");
    std::fs::write(root.path().join("fuzz/inside"), "x").expect("file");

    // A whole second, so the test cannot lose a race to a slow machine.
    let state = type_into(&walker(root.path(), true, 1000), "fuzz");
    assert!(state.at.ends_with("fuzz"), "walked in: {:?}", state.at);
    assert_eq!(state.query, "", "the trailing z was dropped, not typed");
    assert_eq!(state.shown.len(), 1, "the new directory is unfiltered");
}

/// With no deadline the leaked character is exactly the bug the deadline exists for, which is what
/// pins the deadline as the thing doing the work rather than something else.
#[test]
fn without_a_deadline_the_trailing_character_leaks_into_the_new_directory() {
    let root = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir(root.path().join("fuzz")).expect("mkdir");
    std::fs::create_dir(root.path().join("fudge")).expect("mkdir");
    std::fs::write(root.path().join("fuzz/inside"), "x").expect("file");

    let state = type_into(&walker(root.path(), true, 0), "fuzz");
    assert!(state.at.ends_with("fuzz"));
    assert_eq!(state.query, "z", "with no deadline it lands in the filter");
}

/// A single matching *file* is never opened — only directories are walked into.
#[test]
fn one_matching_file_is_left_alone() {
    let root = tempfile::tempdir().expect("tempdir");
    std::fs::write(root.path().join("readme"), "x").expect("file");
    std::fs::create_dir(root.path().join("other")).expect("mkdir");

    let state = type_into(&walker(root.path(), true, 1000), "readme");
    assert_eq!(state.at, root.path().canonicalize().expect("canonical"));
    assert_eq!(state.query, "readme");
}

/// Turned off, the filter narrows and stays put — Enter or Right is then the way in.
#[test]
fn disabled_it_only_filters() {
    let root = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir(root.path().join("fuzz")).expect("mkdir");
    std::fs::create_dir(root.path().join("bench")).expect("mkdir");

    let spec = walker(root.path(), false, 1000);
    let mut state = type_into(&spec, "fuzz");
    assert_eq!(state.at, root.path().canonicalize().expect("canonical"));
    assert_eq!(state.query, "fuzz");
    assert_eq!(state.shown.len(), 1);
    // And the way in still works.
    state.open_selected();
    assert!(state.at.ends_with("fuzz"));
}

/// Arriving somewhere must not walk on by itself: a directory holding exactly one child would
/// otherwise swallow you the moment you got there, before you had typed anything.
#[test]
fn an_empty_query_never_walks_anywhere() {
    let root = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir(root.path().join("only")).expect("mkdir");
    std::fs::create_dir(root.path().join("only/deeper")).expect("mkdir");

    let spec = walker(root.path(), true, 1000);
    let mut state = State::new(&spec);
    state.walk_into_the_only_match(&spec);
    assert_eq!(state.at, root.path().canonicalize().expect("canonical"));
}

/// What `.` does when the key loop calls it: the listing is re-read the other way round.
#[test]
fn toggling_hidden_re_reads_the_directory() {
    let root = tempfile::tempdir().expect("tempdir");
    std::fs::write(root.path().join("plain"), "x").expect("file");
    std::fs::write(root.path().join(".secret"), "x").expect("hidden file");

    let mut state = State::new(&walker(root.path(), true, 1000));
    assert!(!state.entries.iter().any(|e| e.name == ".secret"));

    state.hidden = !state.hidden;
    state.reload(None);
    assert!(state.entries.iter().any(|e| e.name == ".secret"));
    assert!(state.entries.iter().any(|e| e.name == "plain"));

    state.hidden = !state.hidden;
    state.reload(None);
    assert!(!state.entries.iter().any(|e| e.name == ".secret"));
}

//! What the history promises, tested against a real file.
//!
//! Every test opens a store in a temporary directory rather than a fake one, because the claims
//! being made — the order the rows are in, the mode of the file, what survives a command with a
//! newline in it — are properties of what reaches the disk.

use super::*;

fn temp_db() -> (tempfile::TempDir, History) {
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("nested/history.db");
    let history = History::open(&path).expect("the database opens");
    (dir, history)
}

/// The raw value of the first row, for the tests that check what was written rather than what
/// comes back.
fn newest_row(history: &History) -> Option<(Vec<u8>, Vec<u8>)> {
    history.store.read(|reader| {
        reader.find(Tree::History, &Span::all(), |key, value| {
            Some((key.to_vec(), value.to_vec()))
        })
    })
}

/// History is state the user accumulates, not configuration they wrote, so it goes under the
/// data directory — not `$HOME`, and not beside the config.
#[test]
fn the_database_lives_under_the_data_directory() {
    assert_eq!(
        database_path(Some("/x/data"), Some("/home/u")),
        Some(PathBuf::from("/x/data/oslo/history.db"))
    );
    // No XDG: the specification's own default.
    assert_eq!(
        database_path(None, Some("/home/u")),
        Some(PathBuf::from("/home/u/.local/share/oslo/history.db"))
    );
    // An empty XDG is unset, not a relative path from the root.
    assert_eq!(
        database_path(Some("  "), Some("/home/u")),
        Some(PathBuf::from("/home/u/.local/share/oslo/history.db"))
    );
    // Nowhere to put it is not an error; it is a shell without history.
    assert_eq!(database_path(None, None), None);
}

/// The whole reason for a database: the language survives the round trip, so recalling a Lua
/// line while the prompt is in shell mode does not run it as shell.
#[test]
fn a_line_remembers_which_language_it_was_typed_in() {
    let (_dir, history) = temp_db();
    assert!(history.append("ls -la", MODE_SHELL));
    assert!(history.append("print(1)", MODE_LUA));

    let entries = history.recent(10);
    assert_eq!(
        entries,
        vec![
            Entry {
                line: "ls -la".to_string(),
                mode: MODE_SHELL.to_string()
            },
            Entry {
                line: "print(1)".to_string(),
                mode: MODE_LUA.to_string()
            },
        ],
        "oldest first, each with its own mode"
    );
}

/// Opening creates the directory as well as the file — a fresh machine has neither.
#[test]
fn opening_creates_what_is_missing() {
    let (dir, history) = temp_db();
    assert!(dir.path().join("nested/history.db").exists());
    assert!(history.recent(10).is_empty(), "a new database is empty");
}

/// This file is a plaintext record of every command line the shell was told to remember, so it is
/// nobody else's business. It used to be `0664`, which is what a `002` umask makes; the store now
/// tightens both it and the directory it is in.
#[test]
fn the_history_is_private_from_the_moment_it_exists() {
    use std::os::unix::fs::PermissionsExt;

    let (dir, history) = temp_db();
    let mode_of = |path: &Path| {
        std::fs::metadata(path)
            .expect("it exists")
            .permissions()
            .mode()
            & 0o777
    };
    assert!(history.append("passwd", MODE_SHELL));
    assert_eq!(mode_of(&dir.path().join("nested/history.db")), 0o600);
    assert_eq!(mode_of(&dir.path().join("nested")), 0o700);
}

#[test]
fn recent_returns_the_newest_and_trimming_keeps_them() {
    let (_dir, history) = temp_db();
    for i in 1..=20 {
        history.append(&format!("cmd {i}"), MODE_SHELL);
    }
    let last_three = history.recent(3);
    assert_eq!(last_three.len(), 3);
    assert_eq!(last_three[2].line, "cmd 20", "newest is last");
    assert_eq!(last_three[0].line, "cmd 18");

    assert!(history.trim(5));
    let kept = history.recent(100);
    assert_eq!(kept.len(), 5, "trimming leaves the newest");
    assert_eq!(kept[0].line, "cmd 16");

    assert!(history.clear());
    assert!(history.recent(10).is_empty());
}

/// The amortised trim is still a bound: it lets the table run over for a while and then puts
/// it back, rather than taking the file's write lock once per line typed to delete nothing.
#[test]
fn the_bound_is_enforced_in_batches_rather_than_per_line() {
    let (_dir, history) = temp_db();
    for i in 1..=(TRIM_EVERY - 1) {
        history.append(&format!("cmd {i}"), MODE_SHELL);
        history.trim_soon(5);
    }
    assert_eq!(
        history.recent(1000).len(),
        TRIM_EVERY - 1,
        "nothing has been trimmed yet"
    );

    history.append("cmd last", MODE_SHELL);
    history.trim_soon(5);
    let kept = history.recent(1000);
    assert_eq!(kept.len(), 5);
    assert_eq!(kept[4].line, "cmd last", "and it kept the newest");
}

/// `$HISTFILE` is newline-separated and cannot hold this at all. A field is framed rather than
/// separated, so the loop comes back as the one entry it was typed as — and a line with the
/// encoding's own separator in it survives too, which is the case the escape exists for.
#[test]
fn a_command_typed_across_several_lines_comes_back_as_one_entry() {
    let (_dir, history) = temp_db();
    let loop_line = "for f in *.rs; do\n    echo \"$f\"\n done";
    assert!(history.append(loop_line, MODE_SHELL));
    assert!(history.append("printf 'a\u{0}b\\n'", MODE_SHELL));

    let entries = history.recent(10);
    assert_eq!(entries.len(), 2, "two entries, not five lines");
    assert_eq!(
        entries[0].line, loop_line,
        "byte for byte, newlines and all"
    );
    assert_eq!(entries[1].line, "printf 'a\u{0}b\\n'");
}

/// The ordering the module rests on, asserted on the encoding itself: a later line sorts *before*
/// an earlier one, so the last N is the head of the bucket.
#[test]
fn a_newer_line_sorts_before_an_older_one() {
    assert!(slot(9) < slot(8));
    assert!(slot(1) < slot(0));
    assert!(slot(u64::MAX) < slot(FIRST_ID));
    for id in [0, FIRST_ID, 7, u64::MAX] {
        assert_eq!(id_of(&slot(id)), Some(id), "and it reads back");
    }
    assert_eq!(
        id_of(b"short"),
        None,
        "bytes from somewhere else are not ids"
    );
    assert_eq!(id_of(&[0u8; 12]), None, "nor are eight bytes with a tail");
}

/// And the same thing where it matters: the first row of a history of twenty is the twentieth
/// line, so recalling the last one reads one row rather than the whole history.
#[test]
fn the_newest_line_is_the_first_row_of_the_store() {
    let (_dir, history) = temp_db();
    for i in 1..=20 {
        history.append(&format!("cmd {i}"), MODE_SHELL);
    }
    let (key, value) = newest_row(&history).expect("a first row");
    assert_eq!(
        id_of(&key),
        Some(20),
        "twenty lines, and this is the twentieth"
    );
    assert_eq!(decode(&value).expect("a row").line, "cmd 20");
}

/// The third field. Nothing reads it yet — the `at` column it replaces was never selected either —
/// but a `history -t` would need it, and recording it now is what saves a migration later.
#[test]
fn every_line_is_stamped_with_when_it_was_typed() {
    let (_dir, history) = temp_db();
    let before = now();
    assert!(history.append("date", MODE_SHELL));

    let (_key, value) = newest_row(&history).expect("a row");
    let mut fields = Fields::of(&value);
    assert_eq!(fields.text().as_deref(), Some("date"));
    assert_eq!(fields.text().as_deref(), Some(MODE_SHELL));
    let at = fields.int().expect("a timestamp");
    assert!((before..=now()).contains(&at), "{at} is when it was typed");
    assert!(fields.is_empty(), "three fields and nothing else");
}

/// `history -c` empties the store, and the next line typed still lands — the numbering starting
/// over is fine, the store refusing writes afterwards would not be.
#[test]
fn clearing_the_history_leaves_a_store_that_still_takes_lines() {
    let (_dir, history) = temp_db();
    for i in 1..=5 {
        history.append(&format!("cmd {i}"), MODE_SHELL);
    }
    assert!(history.clear());
    assert!(history.recent(10).is_empty());
    assert!(
        history.clear(),
        "clearing an empty history is not a failure"
    );

    assert!(history.append("after", MODE_SHELL));
    assert_eq!(history.recent(10).len(), 1);
    assert_eq!(history.recent(10)[0].line, "after");
}

/// Ids keep climbing across a trim, because the rows a trim drops are the *old* ones and the
/// newest — the one the next id is counted from — is exactly what it keeps.
#[test]
fn trimming_does_not_hand_the_next_line_an_id_that_is_already_taken() {
    let (_dir, history) = temp_db();
    for i in 1..=10 {
        history.append(&format!("cmd {i}"), MODE_SHELL);
    }
    assert!(history.trim(2));
    assert!(history.append("cmd 11", MODE_SHELL));

    let (key, _value) = newest_row(&history).expect("a first row");
    assert_eq!(id_of(&key), Some(11));
    let kept = history.recent(10);
    assert_eq!(kept.len(), 3);
    assert_eq!(kept[2].line, "cmd 11", "and it is still the newest");
}

/// Nothing is asked for, so nothing is read. `$HISTSIZE=0` reaches this.
#[test]
fn asking_for_no_lines_reads_none() {
    let (_dir, history) = temp_db();
    history.append("ls", MODE_SHELL);
    assert!(history.recent(0).is_empty());
    assert!(history.trim(0), "and trimming to nothing empties it");
    assert!(history.recent(10).is_empty());
}

/// Two terminals are the reason the store underneath was chosen, and the id this module hands out
/// is a read of the newest row followed by a write — the shape that loses lines if the two are not
/// one transaction. They are, so a hundred lines typed into two shells at once are a hundred lines
/// under a hundred ids.
///
/// Two `History` values rather than two threads sharing one, because that is what two terminals
/// are: separate opens of the same path, contending on the file's own lock. `flock` belongs to the
/// open file description rather than to the process, so this really does exercise it.
#[test]
fn two_terminals_appending_at_once_lose_no_lines_and_reuse_no_ids() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("history.db");
    History::open(&path).expect("the database opens");

    std::thread::scope(|scope| {
        for terminal in ["one", "two"] {
            let path = path.clone();
            scope.spawn(move || {
                let history = History::open(&path).expect("and opens again, from the other shell");
                for i in 1..=30 {
                    assert!(history.append(&format!("{terminal} {i}"), MODE_SHELL));
                }
            });
        }
    });

    let history = History::open(&path).expect("the database opens");
    let entries = history.recent(1000);
    assert_eq!(entries.len(), 60, "nothing was written over");

    let mut ids: Vec<u64> = history
        .store
        .read(|reader| Some(reader.collect(Tree::History, &Span::all(), |key, _| id_of(key))))
        .expect("the rows are readable");
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), 60, "and every line got an id of its own");
}

/// A file here that this build cannot read — an older oslo's database, or one a disk corrupted —
/// must not cost the shell its history for ever. It starts fresh and the old bytes stay on disk.
///
/// The bytes below are a SQLite header because that is what an older oslo left, but nothing in
/// [`History::open`] knows about SQLite: the test is really "something that is not ours".
#[test]
fn a_history_from_an_older_oslo_is_kept_and_the_shell_starts_fresh() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("history.db");
    let mut foreign = b"SQLite format 3\0".to_vec();
    foreign.resize(16 * 1024, 0);
    std::fs::write(&path, &foreign).expect("written");

    let history = History::open(&path).expect("the shell still gets a history");
    assert!(history.recent(10).is_empty());
    assert!(history.append("ls", MODE_SHELL));
    assert_eq!(history.recent(10).len(), 1);

    let aside = dir.path().join("history.db.unreadable");
    assert_eq!(std::fs::read(&aside).expect("still there"), foreign);
}

/// The bound, at the size where it actually has to work.
///
/// A history at the default `HISTSIZE` is thousands of rows deep, and a hundred lines later the
/// trim has to remove a hundred of them. Doing that in one transaction panics inside jammdb and
/// removes none — measured on this exact shape, which is why [`History::trim`] deletes in chunks.
/// Nothing else in this file is large enough to notice, so if the chunking is ever undone this is
/// the test that says so.
#[test]
fn the_bound_holds_on_a_history_deep_enough_for_one_delete_to_fail() {
    let (_dir, history) = temp_db();
    for i in 1..=3_500 {
        assert!(history.append(&format!("cargo run --example thing-{i}"), MODE_SHELL));
    }
    assert!(history.trim(3_400));

    let kept = history.recent(10_000);
    assert_eq!(kept.len(), 3_400, "a hundred lines really went");
    assert_eq!(
        kept[3_399].line, "cargo run --example thing-3500",
        "the newest stayed"
    );
    assert_eq!(
        kept[0].line, "cargo run --example thing-101",
        "the oldest went"
    );
}

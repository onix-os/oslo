//! What the history promises, tested against a real file.
//!
//! Every test opens a store in a temporary directory rather than a fake one, because the claims
//! being made — the order the rows are in, the mode of the file, what survives a command with a
//! newline in it — are properties of what reaches the disk.

use super::*;
use std::path::Path;

fn temp_db() -> (tempfile::TempDir, crate::track::Track) {
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("nested/history.db");
    let history = crate::track::Track::open(&path).expect("the store opens");
    (dir, history)
}

/// The raw value of the first row, for the tests that check what was written rather than what
/// comes back.
fn newest_row(history: &crate::track::Track) -> Option<(Vec<u8>, Vec<u8>)> {
    history.store.read(|reader| {
        reader.find(Tree::History, &Span::all(), |key, value| {
            Some((key.to_vec(), value.to_vec()))
        })
    })
}

/// The whole reason for a database: the language survives the round trip, so recalling a Lua
/// line while the prompt is in shell mode does not run it as shell.
#[test]
fn a_line_remembers_which_language_it_was_typed_in() {
    let (_dir, history) = temp_db();
    assert!(history.append("ls -la", MODE_SHELL).is_some());
    assert!(history.append("print(1)", MODE_LUA).is_some());

    let entries: Vec<(String, String)> = history
        .recent(10)
        .into_iter()
        .map(|e| (e.line, e.mode))
        .collect();
    assert_eq!(
        entries,
        vec![
            ("ls -la".to_string(), MODE_SHELL.to_string()),
            ("print(1)".to_string(), MODE_LUA.to_string()),
        ],
        "oldest first, each with its own mode"
    );
}

/// The row carries which shell typed it and where in that shell's run it came — the two things a
/// replay needs and a timestamp cannot supply.
///
/// `seq` is what makes an *omission* visible: a secret command is never appended and consumes no
/// id, so the global ordering has no gap for it, and only a per-session counter that skips can
/// show that something was left out.
#[test]
fn a_row_records_the_session_and_its_position_in_it() {
    let (_dir, history) = temp_db();
    history.append("first", MODE_SHELL);
    history.append("second", MODE_SHELL);

    let entries = history.recent(10);
    assert_eq!(entries.len(), 2);
    assert_eq!(
        entries[0].session, entries[1].session,
        "one shell is one session"
    );
    // The ordinal's *value* is not asserted: it is allocated once per process from a `OnceLock`,
    // so in a test binary it belongs to whichever test appended first. What this file can promise
    // is that one shell's rows agree about it.
    //
    // **Increasing, not consecutive**, and that is the contract rather than a concession to the
    // test runner: a secret command is deliberately not appended and still consumes a `seq`, so a
    // gap is the signal, not a defect. Asserting `+ 1` would pin the opposite of what the field is
    // for. (It also happens to be why this was flaky — the counter is per *process*, and a test
    // binary runs many "shells" in one.)
    assert!(
        entries[1].seq > entries[0].seq,
        "the counter advances: {} then {}",
        entries[0].seq,
        entries[1].seq
    );
    assert!(!entries[0].rewritten, "a line nobody rewrote says so");
}

/// A row written before these fields existed still reads. The encoding is appended to, never
/// reordered, so an older store is readable rather than a migration.
#[test]
fn a_row_from_an_older_oslo_reads_back_without_a_session() {
    let (_dir, history) = temp_db();
    // Exactly what the old encoder wrote: line, mode, timestamp, and nothing after it.
    let value = Key::with_capacity(32)
        .text("old line")
        .text(MODE_SHELL)
        .int(1_700_000_000)
        .done();
    history
        .store
        .write(|writer| writer.put(Tree::History, slot(1), value));

    let entries = history.recent(10);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].line, "old line");
    assert_eq!(entries[0].session, 0, "no session recorded, not session 0");
    assert_eq!(entries[0].seq, 0);
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
    assert!(history.append("passwd", MODE_SHELL).is_some());
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
    assert!(history.append(loop_line, MODE_SHELL).is_some());
    assert!(history.append("printf 'a\u{0}b\\n'", MODE_SHELL).is_some());

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
    assert!(history.append("date", MODE_SHELL).is_some());

    let (_key, value) = newest_row(&history).expect("a row");
    let mut fields = Fields::of(&value);
    assert_eq!(fields.text().as_deref(), Some("date"));
    assert_eq!(fields.text().as_deref(), Some(MODE_SHELL));
    let at = fields.int().expect("a timestamp");
    assert!((before..=now()).contains(&at), "{at} is when it was typed");
    // The three that came later. Order is the contract — `decode` reads positionally, so a field
    // inserted rather than appended would silently shift every one after it.
    assert!(fields.int().is_some(), "session");
    assert!(fields.int().is_some(), "seq");
    assert_eq!(fields.int(), Some(0), "rewritten, and this line was not");
    assert!(fields.is_empty(), "six fields and nothing else");
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

    assert!(history.append("after", MODE_SHELL).is_some());
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
    assert!(history.append("cmd 11", MODE_SHELL).is_some());

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
/// Two `History` values use separate handles to the same path, matching two terminals.
#[test]
fn two_terminals_appending_at_once_lose_no_lines_and_reuse_no_ids() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("history.db");
    crate::track::Track::open(&path).expect("the store opens");

    std::thread::scope(|scope| {
        for terminal in ["one", "two"] {
            let path = path.clone();
            scope.spawn(move || {
                let history = crate::track::Track::open(&path)
                    .expect("and opens again, from the other shell");
                for i in 1..=30 {
                    assert!(
                        history
                            .append(&format!("{terminal} {i}"), MODE_SHELL)
                            .is_some()
                    );
                }
            });
        }
    });

    let history = crate::track::Track::open(&path).expect("the store opens");
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

    let history = crate::track::Track::open(&path).expect("the shell still gets a history");
    assert!(history.recent(10).is_empty());
    assert!(history.append("ls", MODE_SHELL).is_some());
    assert_eq!(history.recent(10).len(), 1);

    let aside = dir.path().join("history.db.unreadable");
    assert_eq!(std::fs::read(&aside).expect("still there"), foreign);
}

/// The bound, at the size where it actually has to work.
///
/// A history at the default `HISTSIZE` is thousands of rows deep, and a hundred lines later the
/// trim removes a hundred of them across bounded transactions.
#[test]
fn the_bound_holds_on_a_large_history() {
    let (_dir, history) = temp_db();
    for i in 1..=3_500 {
        assert!(
            history
                .append(&format!("cargo run --example thing-{i}"), MODE_SHELL)
                .is_some()
        );
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

/// **`forget` takes the line out of the log, not only out of the aggregate.**
///
/// It used to delete from `Tree::Run` alone, so the finder's Delete key left the line in
/// `Tree::History` and `recent()` handed it straight back on the next start. The doc comment on
/// `forget` names the case it exists for — "a password on the command line" — and that password
/// survived being forgotten.
///
/// Worth pinning here rather than beside `forget`: this file is the one that knows what the log
/// promises, and the promise is that a line you removed is gone from it.
#[test]
fn forgetting_a_line_removes_it_from_the_log_as_well() {
    let (_dir, history) = temp_db();
    history.append("echo keep", MODE_SHELL);
    history.append("curl -H 'Authorization: Bearer hunter2'", MODE_SHELL);
    history.append("echo keep too", MODE_SHELL);

    let gone = history.forget("curl -H 'Authorization: Bearer hunter2'", MODE_SHELL);
    assert!(gone >= 1, "nothing was deleted");

    let left: Vec<String> = history.recent(10).into_iter().map(|e| e.line).collect();
    assert_eq!(
        left,
        vec!["echo keep".to_string(), "echo keep too".to_string()],
        "the forgotten line is still in the log"
    );
}

/// Only that line, and only in that language. A `forget` that took neighbours with it would be
/// worse than one that took nothing.
#[test]
fn forgetting_leaves_every_other_line_alone() {
    let (_dir, history) = temp_db();
    history.append("same text", MODE_SHELL);
    history.append("same text", MODE_LUA);
    history.append("other", MODE_SHELL);

    history.forget("same text", MODE_SHELL);

    let left: Vec<(String, String)> = history
        .recent(10)
        .into_iter()
        .map(|e| (e.line, e.mode))
        .collect();
    assert_eq!(
        left,
        vec![
            ("same text".to_string(), MODE_LUA.to_string()),
            ("other".to_string(), MODE_SHELL.to_string()),
        ],
        "the Lua line and the unrelated one must survive"
    );
}

/// Every copy of a repeated line goes, not just the newest. Leaving the older ones would make it
/// reappear the moment the newest scrolled out.
#[test]
fn forgetting_removes_every_occurrence() {
    let (_dir, history) = temp_db();
    history.append("repeated", MODE_SHELL);
    history.append("between", MODE_SHELL);
    history.append("repeated", MODE_SHELL);

    history.forget("repeated", MODE_SHELL);

    let left: Vec<String> = history.recent(10).into_iter().map(|e| e.line).collect();
    assert_eq!(left, vec!["between".to_string()]);
}

/// A rule that keeps one link of a chain rewrites the row **in place**, keeping the id.
///
/// The id is what the outcome rows join on, so deleting and re-appending would renumber the line
/// and orphan everything already written against it.
#[test]
fn rewriting_a_line_keeps_its_id_and_says_it_was_rewritten() {
    let (_dir, history) = temp_db();
    let id = history
        .append("aa && bb && cc -c 'd'", MODE_SHELL)
        .expect("an id");

    assert!(history.rewrite_line(id, "cc -c 'd'"));

    let entries = history.recent(10);
    assert_eq!(entries.len(), 1, "still one row, not two");
    assert_eq!(entries[0].line, "cc -c 'd'");
    assert!(
        entries[0].rewritten,
        "a reader must be able to tell this from what was typed"
    );
    assert_eq!(entries[0].mode, MODE_SHELL, "and nothing else moved");
}

/// Rewriting to the same text is not a rewrite. Marking it would claim a transformation that never
/// happened, which is exactly the thing the flag exists to make honest.
#[test]
fn rewriting_a_line_to_itself_does_not_mark_it() {
    let (_dir, history) = temp_db();
    let id = history.append("unchanged", MODE_SHELL).expect("an id");
    assert!(history.rewrite_line(id, "unchanged"));
    assert!(!history.recent(10)[0].rewritten);
}

/// A refused line leaves the log entirely, and takes what it did with it.
#[test]
fn dropping_a_line_removes_it_and_its_outcome() {
    let (_dir, history) = temp_db();
    let kept = history.append("keep", MODE_SHELL).expect("an id");
    let doomed = history.append("forget", MODE_SHELL).expect("an id");
    history.record_outcome(doomed, &[crate::track::Outcome::line(0, Some(0), 1)]);

    assert!(history.drop_line(doomed));

    let left: Vec<String> = history.recent(10).into_iter().map(|e| e.line).collect();
    assert_eq!(left, vec!["keep".to_string()]);
    assert!(
        history.outcome_of(doomed).is_empty(),
        "an outcome nothing can join to is a row nothing will read"
    );
    assert!(history.outcome_of(kept).is_empty());
}

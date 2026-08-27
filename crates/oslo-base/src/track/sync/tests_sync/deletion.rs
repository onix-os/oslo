//! Deleting, and how a deletion travels.
//!
//! Split from [`super`] when that file crossed the line limit. What is here is one subject: a
//! deletion is a *tombstone* — an event that says "this is gone" and has to outlive the thing it
//! deletes, or the next peer to sync would hand the line back. What it must not outlive is the
//! text, which is the whole point of asking for the deletion.

use super::*;

#[test]
fn completion_updates_and_tombstones_propagate_once() {
    let dir = tempfile::tempdir().expect("temp dir");
    let left_path = dir.path().join("left.kv");
    let right_path = dir.path().join("right.kv");
    let left = Track::open(&left_path).expect("left");
    let right = Track::open(&right_path).expect("right");
    let local_id = left.append("echo later", "sh").expect("history id");
    let event_id = left.events(&HistoryFilter::default())[0].id;

    sync_files(&left_path, &right_path, false).expect("incomplete sync");
    assert!(
        right
            .event(event_id)
            .expect("remote event")
            .completion
            .is_none()
    );

    assert!(left.record(&Step {
        ran_in: Visit::at("/left"),
        moved_to: None,
        dwell_ms: 0,
        run: Some(Run {
            argv: "echo later",
            mode: "sh",
            status: Some(0),
            duration_ms: 9,
        }),
    }));
    assert!(left.record_outcome(
        local_id,
        &[Outcome::line(left.current_dir_id(), Some(0), 9)]
    ));
    sync_files(&left_path, &right_path, false).expect("completion sync");
    assert!(
        right
            .event(event_id)
            .expect("remote event")
            .completion
            .is_some()
    );
    assert_eq!(right.commands(10)[0].runs, 1);

    assert_eq!(left.delete_events(&[event_id]).expect("delete"), 1);
    sync_files(&left_path, &right_path, false).expect("delete sync");
    assert!(left.events(&HistoryFilter::default()).is_empty());
    assert!(right.events(&HistoryFilter::default()).is_empty());
    assert!(right.commands(10).is_empty());
    let repeat = sync_files(&left_path, &right_path, false).expect("repeat");
    assert_eq!(repeat.applied_left + repeat.applied_right, 0);
}

/// **A tombstone carries no command.**
///
/// Deleting used to set the flag and keep `line`, so every deleted command survived verbatim in the
/// sync bucket: `history clear --yes` reported success, `history export` printed the passphrase
/// straight back, and the text was still in the file. What a tombstone needs is its identity and
/// its revision — enough for a peer to learn the deletion on the next sync — and nothing a person
/// typed.
#[test]
fn a_deleted_event_keeps_no_text() {
    let dir = tempfile::tempdir().expect("temp dir");
    let track = Track::open(&dir.path().join("hist.db")).expect("store");
    let id = record(&track, "echo my-secret-passphrase", "/w", 0);

    assert_eq!(track.delete_events(&[id]).expect("delete"), 1);

    // The event is still there — a tombstone has to be, or a peer would resurrect the line — but
    // what it holds is a deletion rather than a command.
    let tomb = track.event(id).expect("the tombstone is kept");
    assert!(tomb.deleted, "it is a tombstone");
    assert_eq!(tomb.line, "", "and holds no command");
    assert!(tomb.completion.is_none(), "nor what the command produced");

    // Which is what `export` walks, so nothing there says it either.
    let exported = track.events(&HistoryFilter {
        include_deleted: true,
        ..HistoryFilter::default()
    });
    assert!(
        !exported.iter().any(|e| e.line.contains("passphrase")),
        "nothing exported carries the text"
    );
}

/// Clearing everything is the same promise made about every line at once.
#[test]
fn clearing_leaves_no_text_behind() {
    let dir = tempfile::tempdir().expect("temp dir");
    let track = Track::open(&dir.path().join("hist.db")).expect("store");
    record(&track, "echo first-secret", "/w", 0);
    record(&track, "echo second-secret", "/w", 0);

    assert_eq!(track.clear_events().expect("clear"), 2);
    let all = track.events(&HistoryFilter {
        include_deleted: true,
        ..HistoryFilter::default()
    });
    assert_eq!(all.len(), 2, "the tombstones remain, for the peers");
    assert!(
        all.iter().all(|e| e.line.is_empty()),
        "and none of them holds a command: {:?}",
        all.iter().map(|e| &e.line).collect::<Vec<_>>()
    );
}

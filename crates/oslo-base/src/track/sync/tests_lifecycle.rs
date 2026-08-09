use super::*;
use crate::track::{Run, Step, Track, Visit};

fn record(track: &Track, line: &str, cwd: &str, status: i32) -> EventId {
    let id = track.append(line, "sh").expect("history id");
    assert!(track.record(&Step {
        ran_in: Visit::at(cwd),
        moved_to: None,
        dwell_ms: 0,
        run: Some(Run {
            argv: line,
            mode: "sh",
            status: Some(status),
            duration_ms: 7,
        }),
    }));
    assert!(track.record_outcome(
        id,
        &[Outcome::line(track.current_dir_id(), Some(status), 7)]
    ));
    track.events(&HistoryFilter::default())[0].id
}

#[test]
fn local_drop_reverses_its_aggregate_contribution() {
    let dir = tempfile::tempdir().expect("temp dir");
    let track = Track::open(&dir.path().join("drop.kv")).expect("track");
    let local_id = track.append("echo drop", "sh").expect("history id");
    assert!(track.record(&Step {
        ran_in: Visit::at("/drop"),
        moved_to: None,
        dwell_ms: 0,
        run: Some(Run {
            argv: "echo drop",
            mode: "sh",
            status: Some(0),
            duration_ms: 7,
        }),
    }));
    assert!(track.record_outcome(
        local_id,
        &[Outcome::line(track.current_dir_id(), Some(0), 7)]
    ));
    assert_eq!(track.commands(10)[0].runs, 1);

    assert!(track.drop_line(local_id));
    assert!(track.commands(10).is_empty());
    let mut filter = HistoryFilter {
        include_deleted: true,
        ..HistoryFilter::default()
    };
    filter.limit = Some(1);
    assert!(track.events(&filter)[0].deleted);
}

#[test]
fn append_rewrite_completion_and_segments_project_together() {
    let dir = tempfile::tempdir().expect("temp dir");
    let left_path = dir.path().join("left.kv");
    let right_path = dir.path().join("right.kv");
    let left = Track::open(&left_path).expect("left");
    let right = Track::open(&right_path).expect("right");
    let local_id = left.append("before", "sh").expect("history id");
    let original = left.events(&HistoryFilter::default())[0].clone();
    assert!(left.rewrite_line(local_id, "after"));
    let rewritten = left.event(original.id).expect("rewritten event");
    assert_eq!(rewritten.id, original.id);
    assert_eq!(rewritten.recorded_at, original.recorded_at);
    assert_eq!(rewritten.revision, 2);
    assert!(left.record(&Step {
        ran_in: Visit::at("/work"),
        moved_to: None,
        dwell_ms: 0,
        run: Some(Run {
            argv: "after",
            mode: "sh",
            status: Some(0),
            duration_ms: 9,
        }),
    }));
    assert!(left.record_outcome(
        local_id,
        &[
            Outcome::line(left.current_dir_id(), Some(0), 9),
            Outcome {
                segment: 1,
                join: "&&".to_string(),
                text: "true".to_string(),
                status: Some(0),
                duration_ms: 2,
                dir_id: 0,
            },
        ]
    ));

    sync_files(&left_path, &right_path, false).expect("sync");
    let (observations, places) = right.observations(10);
    assert_eq!(observations.len(), 1);
    assert_eq!(observations[0].line, "after");
    assert!(observations[0].rewritten);
    assert_eq!(observations[0].segments.len(), 1);
    assert_eq!(observations[0].segments[0].text, "true");
    assert_eq!(places[0].path, "/work");
    assert_eq!(right.event(original.id).expect("event").revision, 3);
}

#[test]
fn completed_local_revisions_replace_their_aggregate_contribution() {
    let dir = tempfile::tempdir().expect("temp dir");
    let track = Track::open(&dir.path().join("revisions.kv")).expect("track");
    let local_id = track.append("before", "sh").expect("history id");
    assert!(track.record(&Step {
        ran_in: Visit::at("/work"),
        moved_to: None,
        dwell_ms: 0,
        run: Some(Run {
            argv: "before",
            mode: "sh",
            status: Some(1),
            duration_ms: 100,
        }),
    }));
    assert!(track.record_outcome(
        local_id,
        &[Outcome::line(track.current_dir_id(), Some(1), 100)]
    ));
    assert!(track.rewrite_line(local_id, "after"));
    assert!(track.record_outcome(
        local_id,
        &[Outcome::line(track.current_dir_id(), Some(0), 10)]
    ));

    let commands = track.commands(10);
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].line, "after");
    assert_eq!(commands[0].runs, 1);
    assert!(commands[0].worked);
    let run = track
        .store
        .read(|reader| reader.find(Tree::Run, &Span::all(), |_, value| RunRow::decode(value)))
        .expect("run row");
    assert_eq!(run.fails, 0);
    assert_eq!(run.total_ms, 10);
    assert_eq!(run.max_ms, 10);
}

#[test]
fn the_same_command_from_two_hosts_counts_twice_and_keeps_the_newest_origin() {
    let dir = tempfile::tempdir().expect("temp dir");
    let target = Track::open(&dir.path().join("target.kv")).expect("target");
    let event = |id, host: &str, session: &str, recorded_at| HistoryEvent {
        id: EventId([id; 32]),
        revision: 1,
        deleted: false,
        tie_breaker: [id; 16],
        line: "cargo test".to_string(),
        mode: "sh".to_string(),
        recorded_at,
        host: host.to_string(),
        session: session.to_string(),
        seq: 1,
        rewritten: false,
        completion: Some(HistoryCompletion {
            cwd: "/work".to_string(),
            root: None,
            status: Some(0),
            duration_ms: 5,
            segments: Vec::new(),
        }),
    };

    target
        .import_events(
            &[
                event(1, "host-one", "session-one", 10),
                event(2, "host-two", "session-two", 20),
            ],
            false,
        )
        .expect("import");
    let commands = target.commands(10);
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].runs, 2);
    assert_eq!(commands[0].host, "host-two");
    assert_eq!(commands[0].session, "session-two");
}

#[test]
fn history_filters_and_ordering_combine() {
    let dir = tempfile::tempdir().expect("temp dir");
    let track = Track::open(&dir.path().join("filters.kv")).expect("track");
    let event = |id, line: &str, host: &str, cwd: &str, status, recorded_at| HistoryEvent {
        id: EventId([id; 32]),
        revision: 1,
        deleted: false,
        tie_breaker: [id; 16],
        line: line.to_string(),
        mode: "sh".to_string(),
        recorded_at,
        host: host.to_string(),
        session: format!("session-{id}"),
        seq: 1,
        rewritten: false,
        completion: Some(HistoryCompletion {
            cwd: cwd.to_string(),
            root: None,
            status: Some(status),
            duration_ms: 1,
            segments: Vec::new(),
        }),
    };
    track
        .import_events(
            &[
                event(1, "cargo test", "host-a", "/work", 0, 10),
                event(2, "cargo build", "host-b", "/work", 1, 20),
                event(3, "echo done", "host-a", "/other", 0, 30),
            ],
            false,
        )
        .expect("import");
    let combined = track.events(&HistoryFilter {
        query: Some("cargo".to_string()),
        matching: HistoryMatch::Prefix,
        host: Some("host-a".to_string()),
        cwd: Some("/work".to_string()),
        status: Some(0),
        since: Some(5),
        before: Some(15),
        ..HistoryFilter::default()
    });
    assert_eq!(combined.len(), 1);
    assert_eq!(combined[0].line, "cargo test");
    assert_eq!(
        track
            .events(&HistoryFilter {
                query: Some("cargo test".to_string()),
                matching: HistoryMatch::Exact,
                ..HistoryFilter::default()
            })
            .len(),
        1
    );
    assert_eq!(
        track.events(&HistoryFilter {
            query: Some("build".to_string()),
            limit: Some(1),
            ..HistoryFilter::default()
        })[0]
            .line,
        "cargo build"
    );
}

#[test]
fn clear_and_forget_tombstones_propagate() {
    let dir = tempfile::tempdir().expect("temp dir");
    let left_path = dir.path().join("left.kv");
    let right_path = dir.path().join("right.kv");
    let left = Track::open(&left_path).expect("left");
    let right = Track::open(&right_path).expect("right");
    record(&left, "forget me", "/left", 0);
    record(&left, "clear me", "/left", 0);
    sync_files(&left_path, &right_path, false).expect("initial sync");

    assert!(left.forget("forget me", "sh") > 0);
    assert!(left.clear());
    sync_files(&left_path, &right_path, false).expect("tombstone sync");
    assert!(left.events(&HistoryFilter::default()).is_empty());
    assert!(right.events(&HistoryFilter::default()).is_empty());
    assert!(left.commands(10).is_empty());
    assert!(right.commands(10).is_empty());
    let all = right.events(&HistoryFilter {
        include_deleted: true,
        ..HistoryFilter::default()
    });
    assert_eq!(all.len(), 2);
    assert!(all.iter().all(|event| event.deleted));
}

#[test]
fn local_trim_stays_hidden_without_becoming_a_tombstone() {
    let dir = tempfile::tempdir().expect("temp dir");
    let left_path = dir.path().join("left.kv");
    let right_path = dir.path().join("right.kv");
    let left = Track::open(&left_path).expect("left");
    let right = Track::open(&right_path).expect("right");
    let event_id = record(&left, "trimmed", "/left", 0);

    assert!(left.trim(0));
    assert!(left.recent(10).is_empty());
    assert!(!left.event(event_id).expect("portable event").deleted);
    sync_files(&left_path, &right_path, false).expect("sync");
    assert_eq!(right.recent(10)[0].line, "trimmed");
    sync_files(&left_path, &right_path, false).expect("repeat");
    assert!(left.recent(10).is_empty());
}

#[test]
fn remote_directories_survive_local_missing_path_sweeps() {
    let dir = tempfile::tempdir().expect("temp dir");
    let left_path = dir.path().join("left.kv");
    let right_path = dir.path().join("right.kv");
    let left = Track::open(&left_path).expect("left");
    let right = Track::open(&right_path).expect("right");
    record(&right, "remote", "/path/that/is/not/local", 0);
    sync_files(&left_path, &right_path, false).expect("sync");
    let remote_count = || {
        left.store
            .read(|reader| {
                Some(
                    reader
                        .collect(Tree::Dir, &Span::all(), |_, value| {
                            DirRow::decode(value)?.remote.then_some(())
                        })
                        .len(),
                )
            })
            .unwrap_or_default()
    };
    assert_eq!(remote_count(), 1);
    left.sweep();
    assert_eq!(remote_count(), 1);
    assert_eq!(left.commands(10)[0].line, "remote");
}

#[test]
fn retry_repairs_a_sync_stopped_after_the_first_file_commit() {
    let dir = tempfile::tempdir().expect("temp dir");
    let left_path = dir.path().join("left.kv");
    let right_path = dir.path().join("right.kv");
    let left = Track::open(&left_path).expect("left");
    let right = Track::open(&right_path).expect("right");
    record(&left, "left", "/left", 0);
    record(&right, "right", "/right", 0);
    let left_events = checked_events(&left.store).expect("left events");
    let right_events = checked_events(&right.store).expect("right events");
    let (winners, _) = reconcile(&left_events, &right_events);

    left.store
        .merge_sync_from(&right.store, false, super::admin::destination_event_wins)
        .expect("first bucket merge");
    write_winners(&left.store, &winners).expect("first winner write");
    assert_eq!(left.history_status().pending_projections, 1);
    sync_files(&left_path, &right_path, false).expect("repair sync");
    assert_eq!(left.events(&HistoryFilter::default()).len(), 2);
    assert_eq!(right.events(&HistoryFilter::default()).len(), 2);
    assert_eq!(left.history_status().pending_projections, 0);
    assert_eq!(right.history_status().pending_projections, 0);
    let repeat = sync_files(&left_path, &right_path, false).expect("repeat");
    assert_eq!(repeat.applied_left + repeat.applied_right, 0);
}

#[test]
fn a_same_revision_delete_beats_a_rewrite() {
    let dir = tempfile::tempdir().expect("temp dir");
    let base_path = dir.path().join("base.kv");
    let base = Track::open(&base_path).expect("base");
    let local_id = base.append("before", "sh").expect("history id");
    let event_id = base.events(&HistoryFilter::default())[0].id;
    drop(base);
    let left_path = dir.path().join("left.kv");
    let right_path = dir.path().join("right.kv");
    std::fs::copy(&base_path, &left_path).expect("left copy");
    std::fs::copy(&base_path, &right_path).expect("right copy");
    let left = Track::open(&left_path).expect("left");
    let right = Track::open(&right_path).expect("right");
    assert!(left.rewrite_line(local_id, "rewritten"));
    assert_eq!(right.delete_events(&[event_id]).expect("delete"), 1);
    assert_eq!(left.event(event_id).expect("rewrite").revision, 2);
    assert_eq!(right.event(event_id).expect("delete").revision, 2);

    sync_files(&left_path, &right_path, false).expect("sync");
    assert!(left.event(event_id).expect("left winner").deleted);
    assert!(right.event(event_id).expect("right winner").deleted);
}

#[test]
fn portable_import_is_idempotent_and_backup_verifies() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().expect("temp dir");
    let source_path = dir.path().join("source.kv");
    let target_path = dir.path().join("target.kv");
    let backup_path = dir.path().join("backup.kv");
    let source = Track::open(&source_path).expect("source");
    let target = Track::open(&target_path).expect("target");
    record(&source, "echo portable", "/source", 0);
    let events = source.events(&HistoryFilter::default());

    let first = target.import_events(&events, false).expect("first import");
    assert_eq!(first.added, 1);
    assert_eq!(first.applied, 1);
    let repeated = target.import_events(&events, false).expect("repeat import");
    assert_eq!(repeated.unchanged, 1);
    assert_eq!(repeated.applied, 0);

    let mut newer_stamp = events[0].clone();
    newer_stamp.revision += 1;
    newer_stamp.tie_breaker = [u8::MAX; 16];
    let adopted = target
        .import_events(&[newer_stamp.clone()], false)
        .expect("newer stamp");
    assert_eq!(adopted.unchanged, 1);
    assert_eq!(adopted.applied, 1);
    assert_eq!(
        target.event(newer_stamp.id).expect("adopted").revision,
        newer_stamp.revision
    );
    assert_eq!(
        target
            .import_events(&[newer_stamp], false)
            .expect("repeat stamp")
            .applied,
        0
    );

    target.backup_to(&backup_path).expect("backup");
    let status = verify_file(&backup_path).expect("verified backup");
    assert_eq!(status.visible, 1);
    assert_eq!(
        std::fs::metadata(&backup_path)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    let occupied = dir.path().join("occupied.kv");
    std::fs::write(&occupied, b"keep").expect("occupied fixture");
    assert!(target.backup_to(&occupied).is_err());
    assert_eq!(std::fs::read(&occupied).expect("occupied remains"), b"keep");
}

#[test]
fn administrative_reads_never_replace_or_create_a_file() {
    let dir = tempfile::tempdir().expect("temp dir");
    let missing = dir.path().join("missing.kv");
    assert!(verify_file(&missing).is_err());
    assert!(!missing.exists());

    let invalid = dir.path().join("invalid.kv");
    std::fs::write(&invalid, b"not a database").expect("invalid fixture");
    let before = std::fs::read(&invalid).expect("before");
    assert!(verify_file(&invalid).is_err());
    assert_eq!(std::fs::read(&invalid).expect("after"), before);
    assert!(!dir.path().join("invalid.kv.unreadable").exists());

    let future = dir.path().join("future.kv");
    let track = Track::open(&future).expect("future fixture");
    track.claim_future_version();
    drop(track);
    let before = std::fs::read(&future).expect("before");
    assert_eq!(status_file(&future).expect("future status").schema, 4);
    assert!(verify_file(&future).is_err());
    assert_eq!(std::fs::read(&future).expect("after"), before);
}

#[test]
fn a_partial_schema_migration_resumes_without_duplicates() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("resume.kv");
    let track = Track::open(&path).expect("fixture");
    for line in ["one", "two", "three", "four", "five"] {
        track.append(line, "sh").expect("append");
    }
    let first = track
        .store
        .read(|reader| {
            reader.find(Tree::History, &Span::all(), |key, value| {
                Some((
                    key.to_vec(),
                    value.to_vec(),
                    super::super::log::id_of_key(key)?,
                ))
            })
        })
        .expect("newest row");
    track
        .store
        .write(|writer| {
            writer.clear(Tree::SyncEvent);
            writer.clear(Tree::HistoryEvent);
            writer.clear(Tree::EventProjection);
            migrate_row(writer, &first.0, &first.1)?;
            set_meta(writer, MIGRATION_CURSOR, first.2.saturating_sub(1) as i64)?;
            set_meta(writer, SCHEMA, 2)
        })
        .expect("partial migration");
    drop(track);

    let resumed = Track::open(&path).expect("resume");
    let events = resumed.events(&HistoryFilter::default());
    assert_eq!(events.len(), 5);
    let unique: BTreeSet<EventId> = events.iter().map(|event| event.id).collect();
    assert_eq!(unique.len(), 5);
    assert_eq!(resumed.history_status().schema, 3);
    drop(resumed);
    let reopened = Track::open(&path).expect("idempotent reopen");
    assert_eq!(reopened.events(&HistoryFilter::default()).len(), 5);
}

#[test]
fn malformed_legacy_history_is_not_stamped_as_migrated() {
    let dir = tempfile::tempdir().expect("temp dir");
    let path = dir.path().join("malformed.kv");
    let track = Track::open(&path).expect("fixture");
    let id = track.append("echo ok", "sh").expect("append");
    track
        .store
        .write(|writer| {
            writer.put(Tree::History, super::super::log::slot(id), b"bad".to_vec())?;
            writer.clear(Tree::SyncEvent);
            writer.clear(Tree::HistoryEvent);
            writer.clear(Tree::EventProjection);
            set_meta(writer, SCHEMA, 2)
        })
        .expect("corrupt fixture");
    drop(track);

    assert!(Track::open(&path).is_none());
    let store = Store::open_existing(&path, true).expect("read-only store");
    assert_eq!(store.read(|reader| meta(reader, SCHEMA)), Some(2));
}

#[test]
fn a_failed_dual_write_leaves_neither_representation() {
    let dir = tempfile::tempdir().expect("temp dir");
    let track = Track::open(&dir.path().join("atomic.kv")).expect("track");
    let entry = Entry::new("echo rollback", "sh");
    assert!(
        track
            .store
            .write(|writer| {
                writer.put(
                    Tree::History,
                    super::super::log::slot(1),
                    super::super::log::encode(&entry, 1),
                )?;
                append_local(writer, 1, &entry, 1)?;
                None::<()>
            })
            .is_none()
    );
    let status = track.history_status();
    assert_eq!(status.events, 0);
    assert!(track.recent(10).is_empty());
}

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
fn event_ids_round_trip_as_fixed_lowercase_hex() {
    let id = EventId::random().expect("random id");
    let written = id.to_string();
    assert_eq!(written.len(), 64);
    assert_eq!(written.parse::<EventId>(), Ok(id));
    assert!("abc".parse::<EventId>().is_err());
    assert!("z".repeat(64).parse::<EventId>().is_err());
    assert!("A".repeat(64).parse::<EventId>().is_err());
}

#[test]
fn event_codec_preserves_multiline_and_nul_text() {
    let line = format!("λ\n\0{}", "x".repeat(128 * 1024));
    let mut event = HistoryEvent::local(&Entry::new(&line, ""), 17).expect("event");
    event.completion = Some(HistoryCompletion {
        cwd: "/w/x".to_string(),
        root: Some("/w".to_string()),
        status: Some(0),
        duration_ms: 42,
        segments: vec![HistorySegment {
            segment: 1,
            join: "&&".to_string(),
            text: "echo ok".to_string(),
            status: Some(0),
            duration_ms: 3,
        }],
    });
    assert_eq!(decode_event(event.id, &encode_event(&event)), Some(event));
}

#[test]
fn event_codec_rejects_bad_versions_truncation_and_trailing_data() {
    let event = HistoryEvent::local(&Entry::new("echo ok", "sh"), 17).expect("event");
    let encoded = encode_event(&event);
    for at in 0..encoded.len() {
        assert!(decode_event(event.id, &encoded[..at]).is_none());
    }
    let mut trailing = encoded.clone();
    trailing.push(0);
    assert!(decode_event(event.id, &trailing).is_none());
    let mut zero_revision = event.clone();
    zero_revision.revision = 0;
    assert!(decode_event(event.id, &encode_event(&zero_revision)).is_none());
    assert!(decode_event(event.id, &Key::new().int(2).done()).is_none());
    let invalid_utf8 = Key::new()
        .int(EVENT_FORMAT)
        .int(1)
        .int(0)
        .blob(&[0; 16])
        .int(1)
        .int(1)
        .int(0)
        .blob(&[0xff])
        .text("sh")
        .text("")
        .text("")
        .int(0)
        .done();
    assert!(decode_event(event.id, &invalid_utf8).is_none());
    let empty = HistoryEvent::local(&Entry::new("", "sh"), 17).expect("empty event");
    assert_eq!(decode_event(empty.id, &encode_event(&empty)), Some(empty));
}

#[test]
fn projection_codec_rejects_zero_revisions_and_invalid_flags() {
    let projection = Projection {
        local_id: 1,
        revision: 1,
        deleted: false,
        tie_breaker: [3; 16],
        hidden: false,
        contribution: None,
    };
    assert_eq!(
        decode_projection(&encode_projection(&projection)),
        Some(projection)
    );
    let zero_revision = Key::new()
        .int(PROJECTION_FORMAT)
        .int(1)
        .int(0)
        .int(0)
        .blob(&[3; 16])
        .int(0)
        .int(0)
        .done();
    assert!(decode_projection(&zero_revision).is_none());
    let invalid_flag = Key::new()
        .int(PROJECTION_FORMAT)
        .int(1)
        .int(1)
        .int(2)
        .blob(&[3; 16])
        .int(0)
        .int(0)
        .done();
    assert!(decode_projection(&invalid_flag).is_none());
}

#[test]
fn deletion_wins_a_same_revision_conflict() {
    let live = HistoryEvent::local(&Entry::new("echo ok", "sh"), 17).expect("event");
    let mut deleted = live.clone();
    deleted.deleted = true;
    deleted.tie_breaker = [0; 16];
    assert!(std::ptr::eq(live.preferred(&deleted), &deleted));
}

#[test]
fn legacy_identity_uses_persisted_key_and_value() {
    let one = EventId::legacy(b"key", b"value");
    assert_eq!(one, EventId::legacy(b"key", b"value"));
    assert_ne!(one, EventId::legacy(b"key", b"other"));
}

#[test]
fn divergent_legacy_completions_get_a_deterministic_winner() {
    let id = EventId::legacy(b"key", b"value");
    let entry = Entry::new("echo common", "sh");
    let mut success = HistoryEvent::legacy(id, &entry, 17);
    success.completion = Some(HistoryCompletion {
        cwd: "/work".to_string(),
        root: None,
        status: Some(0),
        duration_ms: 1,
        segments: Vec::new(),
    });
    let mut failure = success.clone();
    failure.completion.as_mut().expect("completion").status = Some(1);
    settle_legacy_tie(&mut success);
    settle_legacy_tie(&mut failure);

    assert_ne!(success.tie_breaker, failure.tie_breaker);
    assert_eq!(success.preferred(&failure), failure.preferred(&success));
}

#[test]
fn disjoint_databases_converge_and_repeat_without_changes() {
    let dir = tempfile::tempdir().expect("temp dir");
    let left_path = dir.path().join("left.kv");
    let right_path = dir.path().join("right.kv");
    let left = Track::open(&left_path).expect("left");
    let right = Track::open(&right_path).expect("right");
    let left_id = record(&left, "echo left", "/left/local", 0);
    let right_id = record(&right, "echo right", "/right/remote", 1);

    let first = sync_files(&left_path, &right_path, false).expect("sync");
    assert_eq!(first.added_left, 1);
    assert_eq!(first.added_right, 1);
    let left_ids: BTreeSet<EventId> = left
        .events(&HistoryFilter::default())
        .into_iter()
        .map(|event| event.id)
        .collect();
    let right_ids: BTreeSet<EventId> = right
        .events(&HistoryFilter::default())
        .into_iter()
        .map(|event| event.id)
        .collect();
    assert_eq!(left_ids, BTreeSet::from([left_id, right_id]));
    assert_eq!(left_ids, right_ids);
    assert!(left.directories_named("remote", "/", 10).is_empty());

    let repeated = sync_files(&left_path, &right_path, false).expect("repeat");
    assert_eq!(
        repeated.added_left + repeated.updated_left + repeated.deleted_left,
        0
    );
    assert_eq!(
        repeated.added_right + repeated.updated_right + repeated.deleted_right,
        0
    );
    assert_eq!(repeated.applied_left, 0);
    assert_eq!(repeated.applied_right, 0);
}

#[test]
fn sync_with_open_handles_and_concurrent_writes_loses_no_events() {
    let dir = tempfile::tempdir().expect("temp dir");
    let left_path = dir.path().join("left.kv");
    let right_path = dir.path().join("right.kv");
    let left = Track::open(&left_path).expect("left");
    let right = Track::open(&right_path).expect("right");
    record(&left, "before", "/left", 0);
    let writer_path = left_path.clone();
    let writer = std::thread::spawn(move || {
        let track = Track::open(&writer_path).expect("writer");
        for number in 0..16 {
            let line = format!("concurrent {number}");
            record(&track, &line, "/left", 0);
        }
    });

    for _ in 0..4 {
        sync_files(&left_path, &right_path, false).expect("live sync");
    }
    writer.join().expect("writer thread");
    sync_files(&left_path, &right_path, false).expect("final sync");
    let left_ids: BTreeSet<EventId> = left
        .events(&HistoryFilter::default())
        .into_iter()
        .map(|event| event.id)
        .collect();
    let right_ids: BTreeSet<EventId> = right
        .events(&HistoryFilter::default())
        .into_iter()
        .map(|event| event.id)
        .collect();
    assert_eq!(left_ids.len(), 17);
    assert_eq!(left_ids, right_ids);
}

#[test]
fn opposite_order_syncs_use_one_canonical_operation_order() {
    let dir = tempfile::tempdir().expect("temp dir");
    let left_path = dir.path().join("left.kv");
    let right_path = dir.path().join("right.kv");
    let left = Track::open(&left_path).expect("left");
    let right = Track::open(&right_path).expect("right");
    record(&left, "left", "/left", 0);
    record(&right, "right", "/right", 0);
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
    let (send, receive) = std::sync::mpsc::channel();
    let spawn = |first: std::path::PathBuf,
                 second: std::path::PathBuf,
                 barrier: std::sync::Arc<std::sync::Barrier>,
                 send: std::sync::mpsc::Sender<Result<SyncReport, String>>| {
        std::thread::spawn(move || {
            barrier.wait();
            let _ = send.send(sync_files(&first, &second, false));
        })
    };
    let first = spawn(
        left_path.clone(),
        right_path.clone(),
        barrier.clone(),
        send.clone(),
    );
    let second = spawn(right_path.clone(), left_path.clone(), barrier.clone(), send);
    barrier.wait();
    for _ in 0..2 {
        receive
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("sync did not deadlock")
            .expect("sync succeeds");
    }
    first.join().expect("first sync");
    second.join().expect("second sync");
    sync_files(&left_path, &right_path, false).expect("final sync");
    assert_eq!(left.events(&HistoryFilter::default()).len(), 2);
    assert_eq!(right.events(&HistoryFilter::default()).len(), 2);
}

#[test]
fn reversed_arguments_keep_report_sides_with_the_arguments() {
    let dir = tempfile::tempdir().expect("temp dir");
    let left_path = dir.path().join("left.kv");
    let right_path = dir.path().join("right.kv");
    let left = Track::open(&left_path).expect("left");
    let _right = Track::open(&right_path).expect("right");
    record(&left, "left", "/left", 0);

    let report = sync_files(&right_path, &left_path, true).expect("reverse dry run");
    assert_eq!(report.added_left, 1);
    assert_eq!(report.added_right, 0);
}

#[test]
fn stale_sync_work_never_replaces_a_concurrent_revision() {
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
    let stale = checked_events(&left.store).expect("stale snapshot");

    assert!(right.rewrite_line(local_id, "concurrent"));
    right
        .store
        .merge_sync_from(&left.store, true, super::admin::destination_event_wins)
        .expect("protected overwrite merge");
    write_winners(&right.store, &stale).expect("protected stale winner write");

    let event = right.event(event_id).expect("event survives");
    assert_eq!(event.line, "concurrent");
    assert_eq!(event.revision, 2);
}

#[test]
fn independent_local_ids_do_not_collide() {
    let dir = tempfile::tempdir().expect("temp dir");
    let left_path = dir.path().join("left.kv");
    let right_path = dir.path().join("right.kv");
    let left = Track::open(&left_path).expect("left");
    let right = Track::open(&right_path).expect("right");
    record(&left, "same local id on left", "/same/path", 0);
    record(&right, "same local id on right", "/same/path", 0);

    sync_files(&left_path, &right_path, false).expect("sync");
    let lines: BTreeSet<String> = left
        .events(&HistoryFilter::default())
        .into_iter()
        .map(|event| event.line)
        .collect();
    assert_eq!(
        lines,
        BTreeSet::from([
            "same local id on left".to_string(),
            "same local id on right".to_string(),
        ])
    );
}

#[test]
fn dry_run_and_same_file_checks_do_not_write() {
    let dir = tempfile::tempdir().expect("temp dir");
    let left_path = dir.path().join("left.kv");
    let right_path = dir.path().join("right.kv");
    let left = Track::open(&left_path).expect("left");
    let right = Track::open(&right_path).expect("right");
    record(&left, "left", "/left", 0);
    record(&right, "right", "/right", 0);
    let before_left = std::fs::read(&left_path).expect("left bytes");
    let before_right = std::fs::read(&right_path).expect("right bytes");

    let report = sync_files(&left_path, &right_path, true).expect("dry run");
    assert_eq!(report.added_left, 1);
    assert_eq!(report.added_right, 1);
    assert_eq!(std::fs::read(&left_path).expect("left bytes"), before_left);
    assert_eq!(
        std::fs::read(&right_path).expect("right bytes"),
        before_right
    );
    assert!(sync_files(&left_path, &left_path, false).is_err());
    let link = dir.path().join("left-link.kv");
    std::fs::hard_link(&left_path, &link).expect("hard link");
    assert!(sync_files(&left_path, &link, false).is_err());
    let symlink = dir.path().join("left-symlink.kv");
    std::os::unix::fs::symlink(&left_path, &symlink).expect("symlink");
    assert!(sync_files(&left_path, &symlink, false).is_err());
    let missing = dir.path().join("missing.kv");
    assert!(sync_files(&left_path, &missing, false).is_err());
    assert!(!missing.exists());
}

#[test]
fn dry_run_preserves_modes_and_sync_makes_databases_private() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().expect("temp dir");
    let left_path = dir.path().join("left.kv");
    let right_path = dir.path().join("right.kv");
    let left = Track::open(&left_path).expect("left");
    let right = Track::open(&right_path).expect("right");
    record(&left, "left", "/left", 0);
    record(&right, "right", "/right", 0);
    std::fs::set_permissions(&left_path, std::fs::Permissions::from_mode(0o644))
        .expect("left mode");
    std::fs::set_permissions(&right_path, std::fs::Permissions::from_mode(0o644))
        .expect("right mode");

    sync_files(&left_path, &right_path, true).expect("dry run");
    assert_eq!(
        std::fs::metadata(&left_path)
            .expect("left metadata")
            .permissions()
            .mode()
            & 0o777,
        0o644
    );
    let invalid = dir.path().join("z-invalid.kv");
    std::fs::write(&invalid, b"not a database").expect("invalid fixture");
    assert!(sync_files(&left_path, &invalid, false).is_err());
    assert_eq!(
        std::fs::metadata(&left_path)
            .expect("left metadata")
            .permissions()
            .mode()
            & 0o777,
        0o644
    );
    sync_files(&left_path, &right_path, false).expect("sync");
    for path in [&left_path, &right_path] {
        assert_eq!(
            std::fs::metadata(path)
                .expect("metadata")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}

#[test]
fn a_permissions_failure_leaves_both_databases_unchanged() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().expect("temp dir");
    let left_path = dir.path().join("left.kv");
    let right_path = dir.path().join("right.kv");
    let left = Track::open(&left_path).expect("left");
    let right = Track::open(&right_path).expect("right");
    record(&left, "left", "/left", 0);
    record(&right, "right", "/right", 0);
    drop(left);
    drop(right);
    let before_left = std::fs::read(&left_path).expect("left bytes");
    let before_right = std::fs::read(&right_path).expect("right bytes");
    std::fs::set_permissions(&right_path, std::fs::Permissions::from_mode(0o000))
        .expect("restrict right");

    let result = sync_files(&left_path, &right_path, false);
    std::fs::set_permissions(&right_path, std::fs::Permissions::from_mode(0o600))
        .expect("restore right");
    assert!(result.is_err());
    assert_eq!(std::fs::read(&left_path).expect("left after"), before_left);
    assert_eq!(
        std::fs::read(&right_path).expect("right after"),
        before_right
    );
}

#[test]
fn copied_schema_two_history_gets_the_same_legacy_identity() {
    let dir = tempfile::tempdir().expect("temp dir");
    let source_path = dir.path().join("source.kv");
    let source = Track::open(&source_path).expect("source");
    record(&source, "echo common", "/common", 0);
    source
        .store
        .write(|writer| {
            writer.clear(Tree::SyncEvent);
            writer.clear(Tree::HistoryEvent);
            writer.clear(Tree::EventProjection);
            set_meta(writer, SCHEMA, 2)
        })
        .expect("downgrade fixture");
    drop(source);
    let left_path = dir.path().join("left.kv");
    let right_path = dir.path().join("right.kv");
    std::fs::copy(&source_path, &left_path).expect("left copy");
    std::fs::copy(&source_path, &right_path).expect("right copy");

    let left = Track::open(&left_path).expect("left migrates");
    let right = Track::open(&right_path).expect("right migrates");
    let left_event = left.events(&HistoryFilter::default());
    let right_event = right.events(&HistoryFilter::default());
    assert_eq!(left_event.len(), 1);
    assert_eq!(left_event[0].id, right_event[0].id);
    assert_eq!(left.history_status().schema, 3);
    assert_eq!(right.history_status().schema, 3);
}

#[test]
fn a_common_base_diverges_without_duplicating_the_common_event() {
    let dir = tempfile::tempdir().expect("temp dir");
    let base_path = dir.path().join("base.kv");
    let base = Track::open(&base_path).expect("base");
    let common = record(&base, "echo common", "/base", 0);
    drop(base);
    let left_path = dir.path().join("left.kv");
    let right_path = dir.path().join("right.kv");
    std::fs::copy(&base_path, &left_path).expect("left copy");
    std::fs::copy(&base_path, &right_path).expect("right copy");
    let left = Track::open(&left_path).expect("left");
    let right = Track::open(&right_path).expect("right");
    record(&left, "echo left", "/left", 0);
    record(&right, "echo right", "/right", 0);

    sync_files(&left_path, &right_path, false).expect("sync");
    let events = left.events(&HistoryFilter::default());
    assert_eq!(events.len(), 3);
    assert_eq!(events.iter().filter(|event| event.id == common).count(), 1);
    assert_eq!(right.events(&HistoryFilter::default()).len(), 3);
}

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

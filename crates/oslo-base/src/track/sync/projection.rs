use super::*;
use crate::track::{log, outcome, redact};

pub(in crate::track) fn append_local(
    writer: &Writer<'_, '_>,
    local_id: u64,
    entry: &Entry,
    recorded_at: u64,
) -> Option<()> {
    let event = HistoryEvent::local(entry, recorded_at)?;
    put_event(writer, &event)?;
    writer.put(Tree::HistoryEvent, local_key(local_id), event.id.0.to_vec())?;
    put_projection(writer, event.id, &Projection::for_local(local_id, &event))
}

pub(in crate::track) fn rewrite_local(
    writer: &Writer<'_, '_>,
    local_id: u64,
    line: &str,
) -> Option<()> {
    let id = id_for_local(writer, local_id)?;
    let mut event = event_of(writer, id)?;
    if event.line == line {
        return Some(());
    }
    event.line = line.to_string();
    event.rewritten = true;
    event.advance()?;
    put_event(writer, &event)?;
    let mut projection = projection_of(writer, id)?;
    if let Some(previous) = projection.contribution.take() {
        let dir_id = previous.dir_id;
        remove_contribution(writer, &previous)?;
        projection.contribution = contribution_for(&event, dir_id);
        if let Some(contribution) = &projection.contribution {
            add_contribution(writer, contribution)?;
        }
    }
    projection.revision = event.revision;
    projection.deleted = false;
    projection.tie_breaker = event.tie_breaker;
    put_projection(writer, id, &projection)
}

pub(in crate::track) fn complete_local(
    writer: &Writer<'_, '_>,
    local_id: u64,
    rows: &[Outcome],
) -> Option<()> {
    let id = id_for_local(writer, local_id)?;
    let mut event = event_of(writer, id)?;
    let Some(line) = rows.iter().find(|row| row.segment == 0) else {
        return Some(());
    };
    let dir = read_dir(writer, line.dir_id);
    event.completion = Some(HistoryCompletion {
        cwd: dir.as_ref().map(|dir| dir.path.clone()).unwrap_or_default(),
        root: dir.and_then(|dir| dir.root),
        status: line.status,
        duration_ms: line.duration_ms,
        segments: rows
            .iter()
            .filter(|row| row.segment != 0)
            .map(|row| HistorySegment {
                segment: row.segment,
                join: row.join.clone(),
                text: row.text.clone(),
                status: row.status,
                duration_ms: row.duration_ms,
            })
            .collect(),
    });
    event.advance()?;
    put_event(writer, &event)?;
    let mut projection = projection_of(writer, id)?;
    if let Some(previous) = projection.contribution.take() {
        remove_contribution(writer, &previous)?;
        let next = contribution_for(&event, line.dir_id)?;
        add_contribution(writer, &next)?;
        projection.contribution = Some(next);
    } else {
        projection.contribution = contribution_for(&event, line.dir_id)
            .filter(|run| writer.has(Tree::Run, &key::run(run.dir_id, &run.mode, &run.argv)));
    }
    projection.revision = event.revision;
    projection.deleted = false;
    projection.tie_breaker = event.tie_breaker;
    put_projection(writer, id, &projection)
}

pub(in crate::track) fn tombstone_local(writer: &Writer<'_, '_>, local_id: u64) -> Option<()> {
    let id = id_for_local(writer, local_id)?;
    let mut event = event_of(writer, id)?;
    if !event.deleted {
        event.deleted = true;
        // A tombstone carries no command — see the same clearing in `admin::delete_events`. The
        // flag alone left every deleted line verbatim in the sync bucket, where `history export`
        // printed it straight back.
        event.line = String::new();
        event.completion = None;
        event.advance()?;
        put_event(writer, &event)?;
    }
    let mut projection = projection_of(writer, id)?;
    if let Some(previous) = projection.contribution.take() {
        remove_contribution(writer, &previous)?;
    }
    projection.revision = event.revision;
    projection.deleted = true;
    projection.tie_breaker = event.tie_breaker;
    projection.hidden = true;
    put_projection(writer, id, &projection)
}

pub(in crate::track) fn hide_local(writer: &Writer<'_, '_>, local_id: u64) -> Option<()> {
    let id = id_for_local(writer, local_id)?;
    let mut projection = projection_of(writer, id)?;
    projection.hidden = true;
    put_projection(writer, id, &projection)
}

fn contribution_for(event: &HistoryEvent, dir_id: u64) -> Option<Contribution> {
    let completion = event.completion.as_ref()?;
    let (argv, head) = redact::prepare(&event.line);
    (!argv.is_empty()).then_some(Contribution {
        dir_id,
        mode: event.mode.clone(),
        argv,
        head,
        at: event.recorded_at as i64,
        status: completion.status.map(i64::from),
        duration_ms: capped(completion.duration_ms),
        session: event.session.clone(),
        host: event.host.clone(),
    })
}

fn row_for(contribution: &Contribution) -> RunRow {
    RunRow {
        head: contribution.head.clone(),
        runs: 1,
        fails: i64::from(contribution.status.is_some_and(|status| status != 0)),
        last_at: contribution.at,
        last_status: contribution.status,
        total_ms: contribution.duration_ms,
        max_ms: contribution.duration_ms,
        session: contribution.session.clone(),
        host: contribution.host.clone(),
    }
}

fn add_contribution(writer: &Writer<'_, '_>, contribution: &Contribution) -> Option<()> {
    let primary = key::run(contribution.dir_id, &contribution.mode, &contribution.argv);
    match writer
        .get(Tree::Run, &primary)
        .and_then(|value| RunRow::decode(&value))
    {
        Some(mut row) => {
            let next = row_for(contribution);
            if next.last_at >= row.last_at {
                row.absorb(&next);
            } else {
                row.runs += 1;
                row.fails += next.fails;
                row.total_ms += next.total_ms;
                row.max_ms = row.max_ms.max(next.max_ms);
            }
            writer.put(Tree::Run, primary, row.encode())
        }
        None => {
            writer.put(Tree::Run, primary, row_for(contribution).encode())?;
            writer.put(
                Tree::RunByArgv,
                key::by_argv(&contribution.mode, &contribution.argv, contribution.dir_id),
                Vec::new(),
            )
        }
    }
}

fn remove_contribution(writer: &Writer<'_, '_>, contribution: &Contribution) -> Option<()> {
    let primary = key::run(contribution.dir_id, &contribution.mode, &contribution.argv);
    let Some(mut row) = writer
        .get(Tree::Run, &primary)
        .and_then(|value| RunRow::decode(&value))
    else {
        return Some(());
    };
    row.runs = row.runs.saturating_sub(1);
    row.fails = row.fails.saturating_sub(i64::from(
        contribution.status.is_some_and(|status| status != 0),
    ));
    row.total_ms = row.total_ms.saturating_sub(contribution.duration_ms);
    if row.runs <= 0 {
        writer.delete(Tree::Run, &primary);
        writer.delete(
            Tree::RunByArgv,
            &key::by_argv(&contribution.mode, &contribution.argv, contribution.dir_id),
        );
        return Some(());
    }
    writer.put(Tree::Run, primary, row.encode())
}

fn imported_dir(writer: &Writer<'_, '_>, event: &HistoryEvent) -> Option<u64> {
    let done = event.completion.as_ref()?;
    if let Some(found) = writer.find(Tree::Dir, &Span::all(), |key, value| {
        let mut fields = Fields::of(key);
        let id = fields.int()?;
        let row = DirRow::decode(value)?;
        (row.remote && row.host == event.host && row.path == done.cwd).then_some(id)
    }) {
        return Some(found);
    }
    insert_imported_dir(
        writer,
        &DirRow::imported(&done.cwd, done.root.as_deref(), &event.host),
    )
}

pub(in crate::track) fn apply_event(writer: &Writer<'_, '_>, event: &HistoryEvent) -> Option<bool> {
    let mut projection = projection_of(writer, event.id).unwrap_or(Projection {
        local_id: 0,
        revision: 0,
        deleted: false,
        tie_breaker: [0; 16],
        hidden: false,
        contribution: None,
    });
    if projection.stamp() == event.stamp() {
        return Some(false);
    }
    if let Some(previous) = projection.contribution.take() {
        remove_contribution(writer, &previous)?;
    }
    if event.deleted {
        if projection.local_id != 0 {
            writer.delete(Tree::History, &log::slot(projection.local_id));
            writer.delete_span(Tree::Outcome, &outcome::span_of(projection.local_id));
        }
        projection.hidden = true;
    } else {
        if projection.local_id == 0 {
            projection.local_id = log::next_id(writer);
            writer.put(
                Tree::HistoryEvent,
                local_key(projection.local_id),
                event.id.0.to_vec(),
            )?;
        }
        if !projection.hidden {
            let entry = Entry {
                line: event.line.clone(),
                mode: event.mode.clone(),
                session: 0,
                seq: event.seq,
                rewritten: event.rewritten,
            };
            writer.put(
                Tree::History,
                log::slot(projection.local_id),
                log::encode(&entry, event.recorded_at),
            )?;
            writer.delete_span(Tree::Outcome, &outcome::span_of(projection.local_id));
        }
        if let Some(done) = &event.completion {
            let dir_id = imported_dir(writer, event)?;
            if !projection.hidden {
                let line = Outcome::line(dir_id, done.status, done.duration_ms);
                writer.put(
                    Tree::Outcome,
                    outcome::slot(projection.local_id, 0),
                    outcome::encode(&line),
                )?;
                for segment in &done.segments {
                    let row = Outcome {
                        segment: segment.segment,
                        join: segment.join.clone(),
                        text: segment.text.clone(),
                        status: segment.status,
                        duration_ms: segment.duration_ms,
                        dir_id: 0,
                    };
                    writer.put(
                        Tree::Outcome,
                        outcome::slot(projection.local_id, row.segment),
                        outcome::encode(&row),
                    )?;
                }
            }
            projection.contribution = contribution_for(event, dir_id);
            if let Some(contribution) = &projection.contribution {
                add_contribution(writer, contribution)?;
            }
        }
    }
    projection.revision = event.revision;
    projection.deleted = event.deleted;
    projection.tie_breaker = event.tie_breaker;
    put_projection(writer, event.id, &projection)?;
    Some(true)
}

pub(in crate::track) fn migrate(store: &Store, found: i64) -> Option<()> {
    if found >= SCHEMA_VERSION {
        return Some(());
    }
    let mut before = store
        .read(|reader| meta(reader, MIGRATION_CURSOR))
        .unwrap_or(i64::MAX)
        .max(0) as u64;
    loop {
        let batch = store.read(|reader| {
            let mut rows = Vec::with_capacity(MIGRATION_BATCH);
            let mut valid = true;
            reader.scan(Tree::History, &Span::all(), |key, value| {
                let Some(id) = log::id_of_key(key) else {
                    valid = false;
                    return Walk::Stop;
                };
                if id > before || reader.has(Tree::HistoryEvent, &local_key(id)) {
                    return Walk::On;
                }
                rows.push((key.to_vec(), value.to_vec()));
                if rows.len() >= MIGRATION_BATCH {
                    Walk::Stop
                } else {
                    Walk::On
                }
            });
            valid.then_some(rows)
        })?;
        if batch.is_empty() {
            return store.write(|writer| {
                writer.delete(Tree::Meta, &Key::new().text(MIGRATION_CURSOR).done());
                set_meta(writer, SCHEMA, SCHEMA_VERSION)
            });
        }
        before = batch
            .last()
            .and_then(|(key, _)| log::id_of_key(key))
            .unwrap_or(1)
            .saturating_sub(1);
        store.write(|writer| {
            for (history_key, history_value) in &batch {
                migrate_row(writer, history_key, history_value)?;
            }
            set_meta(writer, MIGRATION_CURSOR, before as i64)
        })?;
    }
}

pub(super) fn migrate_row(
    writer: &Writer<'_, '_>,
    history_key: &[u8],
    history_value: &[u8],
) -> Option<()> {
    let (local_id, entry, recorded_at) = log::stored_entry(history_key, history_value)?;
    let id = EventId::legacy(history_key, history_value);
    let mut event = HistoryEvent::legacy(id, &entry, recorded_at);
    let outcomes = writer.collect(Tree::Outcome, &outcome::span_of(local_id), outcome::decode);
    let line = outcomes.iter().find(|row| row.segment == 0);
    if let Some(line) = line {
        let dir = read_dir(writer, line.dir_id)?;
        event.completion = Some(HistoryCompletion {
            cwd: dir.path,
            root: dir.root,
            status: line.status,
            duration_ms: line.duration_ms,
            segments: outcomes
                .iter()
                .filter(|row| row.segment != 0)
                .map(|row| HistorySegment {
                    segment: row.segment,
                    join: row.join.clone(),
                    text: row.text.clone(),
                    status: row.status,
                    duration_ms: row.duration_ms,
                })
                .collect(),
        });
    }
    settle_legacy_tie(&mut event);
    put_event(writer, &event)?;
    writer.put(Tree::HistoryEvent, local_key(local_id), id.0.to_vec())?;
    let mut projection = Projection::for_local(local_id, &event);
    if let Some(line) = line {
        projection.contribution = contribution_for(&event, line.dir_id)
            .filter(|run| writer.has(Tree::Run, &key::run(run.dir_id, &run.mode, &run.argv)));
    }
    put_projection(writer, id, &projection)
}

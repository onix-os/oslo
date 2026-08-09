use super::projection::apply_event;
use super::*;
use crate::track::Track;

impl Track {
    pub fn events(&self, filter: &HistoryFilter) -> Vec<HistoryEvent> {
        let mut events = self
            .store
            .read(|reader| {
                Some(reader.collect(Tree::SyncEvent, &Span::all(), |key, value| {
                    let id = EventId(key.try_into().ok()?);
                    let event = decode_event(id, value)?;
                    matches_filter(&event, filter).then_some(event)
                }))
            })
            .unwrap_or_default();
        events.sort_by(|left, right| {
            let order = left
                .recorded_at
                .cmp(&right.recorded_at)
                .then(left.id.cmp(&right.id));
            if filter.oldest_first {
                order
            } else {
                order.reverse()
            }
        });
        if let Some(limit) = filter.limit {
            events.truncate(limit);
        }
        events
    }

    pub fn event(&self, id: EventId) -> Option<HistoryEvent> {
        self.store
            .read(|reader| decode_event(id, &reader.get(Tree::SyncEvent, &event_key(id))?))
    }

    pub fn history_status(&self) -> HistoryStatus {
        history_status_of(&self.store).unwrap_or_default()
    }

    pub fn delete_events(&self, ids: &[EventId]) -> Result<usize, String> {
        if !self.writable {
            return Err("history database is read-only".to_string());
        }
        let mut deleted = 0;
        for chunk in ids.chunks(MIGRATION_BATCH) {
            deleted += self.store.write_checked(|writer| {
                let mut changed = 0;
                for id in chunk {
                    let Some(mut event) = event_of(writer, *id) else {
                        continue;
                    };
                    if event.deleted {
                        continue;
                    }
                    event.deleted = true;
                    event
                        .advance()
                        .ok_or_else(|| "cannot create an event revision".to_string())?;
                    put_event(writer, &event)
                        .ok_or_else(|| "cannot write a history tombstone".to_string())?;
                    apply_event(writer, &event)
                        .ok_or_else(|| "cannot apply a history tombstone".to_string())?;
                    changed += 1;
                }
                Ok(changed)
            })?;
        }
        Ok(deleted)
    }

    pub fn clear_events(&self) -> Result<usize, String> {
        let ids: Vec<EventId> = self
            .events(&HistoryFilter::default())
            .into_iter()
            .map(|event| event.id)
            .collect();
        self.delete_events(&ids)
    }

    pub fn import_events(
        &self,
        incoming: &[HistoryEvent],
        dry_run: bool,
    ) -> Result<ImportReport, String> {
        let mut report = ImportReport::default();
        let mut winners = Vec::new();
        let mut unique: BTreeMap<EventId, HistoryEvent> = BTreeMap::new();
        for event in incoming {
            if event.revision == 0
                || decode_event(event.id, &encode_event(event)).as_ref() != Some(event)
            {
                return Err(format!("history event {} is invalid", event.id));
            }
            unique
                .entry(event.id)
                .and_modify(|current| *current = current.preferred(event).clone())
                .or_insert_with(|| event.clone());
        }
        for event in unique.values() {
            match self.event(event.id) {
                None => {
                    if event.deleted {
                        report.deleted += 1;
                    } else {
                        report.added += 1;
                    }
                    winners.push(event.clone());
                }
                Some(current) => {
                    let winner = current.preferred(event);
                    if current.same_payload(event) {
                        report.unchanged += 1;
                        if winner != &current {
                            winners.push(winner.clone());
                        }
                    } else if winner == &current {
                        report.unchanged += 1;
                    } else {
                        if winner.deleted && !current.deleted {
                            report.deleted += 1;
                        } else {
                            report.updated += 1;
                        }
                        winners.push(winner.clone());
                    }
                }
            }
        }
        if dry_run {
            return Ok(report);
        }
        if !self.writable {
            return Err("history database is read-only".to_string());
        }
        for chunk in winners.chunks(MIGRATION_BATCH) {
            report.applied += self.store.write_checked(|writer| {
                let mut applied = 0;
                for event in chunk {
                    put_event(writer, event)
                        .ok_or_else(|| format!("cannot write history event {}", event.id))?;
                    applied += usize::from(
                        apply_event(writer, event)
                            .ok_or_else(|| format!("cannot project history event {}", event.id))?,
                    );
                }
                Ok(applied)
            })?;
        }
        Ok(report)
    }

    pub fn backup_to(&self, destination: &Path) -> Result<(), String> {
        self.store.backup_to(destination)
    }
}

pub fn verify_file(path: &Path) -> Result<HistoryStatus, String> {
    let store = Store::open_existing(path, true)?;
    store.verify()?;
    let status = history_status_of(&store)?;
    if status.schema != SCHEMA_VERSION {
        return Err(format!(
            "{}: schema {} is not supported; expected {SCHEMA_VERSION}",
            path.display(),
            status.schema
        ));
    }
    checked_events(&store)?;
    Ok(status)
}

pub fn status_file(path: &Path) -> Result<HistoryStatus, String> {
    let store = Store::open_existing(path, true)?;
    history_status_of(&store)
}

fn history_status_of(store: &Store) -> Result<HistoryStatus, String> {
    let stats = store.stats()?;
    store.read_checked(|reader| {
        let events = reader.count(Tree::SyncEvent, &Span::all());
        let tombstones = reader.collect(Tree::SyncEvent, &Span::all(), |key, value| {
            let id = EventId(key.try_into().ok()?);
            decode_event(id, value)?.deleted.then_some(())
        });
        let pending_projections = reader
            .collect(Tree::SyncEvent, &Span::all(), |key, value| {
                let id = EventId(key.try_into().ok()?);
                let event = decode_event(id, value)?;
                let applied = reader
                    .get(Tree::EventProjection, &event_key(id))
                    .and_then(|value| decode_projection(&value))
                    .is_some_and(|projection| projection.stamp() == event.stamp());
                (!applied).then_some(())
            })
            .len();
        Ok(HistoryStatus {
            path: store.path().display().to_string(),
            schema: meta(reader, SCHEMA).unwrap_or(0),
            file_size: store.size(),
            events,
            visible: events.saturating_sub(tombstones.len()),
            tombstones: tombstones.len(),
            projections: reader.count(Tree::EventProjection, &Span::all()),
            pending_projections,
            page_size: stats.page_size,
            allocated_pages: stats.allocated_pages,
            free_pages: stats.free_pages,
            pending_pages: stats.pending_pages,
            active_readers: stats.active_readers,
        })
    })
}

pub fn sync_files(
    left_path: &Path,
    right_path: &Path,
    dry_run: bool,
) -> Result<SyncReport, String> {
    let (left_canonical, right_canonical) = distinct_paths(left_path, right_path)?;
    let swapped = left_canonical > right_canonical;
    let (first_path, second_path) = if swapped {
        (right_path, left_path)
    } else {
        (left_path, right_path)
    };
    let first = Track::open_existing_unmodified(first_path, dry_run)?;
    let second = Track::open_existing_unmodified(second_path, dry_run)?;
    first.store.verify()?;
    second.store.verify()?;
    let first_events = checked_events(&first.store)?;
    let second_events = checked_events(&second.store)?;
    let (winners, mut report) = reconcile(&first_events, &second_events);
    if dry_run {
        return Ok(orient_report(report, swapped));
    }

    first.store.ensure_private()?;
    second.store.ensure_private()?;
    first
        .store
        .merge_sync_from(&second.store, false, destination_event_wins)?;
    write_winners(&first.store, &winners)?;
    second
        .store
        .merge_sync_from(&first.store, true, destination_event_wins)?;
    write_winners(&first.store, &checked_events(&second.store)?)?;
    write_winners(&second.store, &checked_events(&first.store)?)?;
    report.applied_left = apply_winners(&first.store, &checked_events(&first.store)?)?;
    report.applied_right = apply_winners(&second.store, &checked_events(&second.store)?)?;
    Ok(orient_report(report, swapped))
}

pub(super) fn destination_event_wins(
    key: &[u8],
    destination: &[u8],
    source: &[u8],
) -> Result<bool, String> {
    let id = EventId(
        key.try_into()
            .map_err(|_| "history event key is not 32 bytes".to_string())?,
    );
    let destination =
        decode_event(id, destination).ok_or_else(|| format!("history event {id} is malformed"))?;
    let source =
        decode_event(id, source).ok_or_else(|| format!("history event {id} is malformed"))?;
    Ok(destination.preferred(&source) == &destination)
}

fn orient_report(mut report: SyncReport, swapped: bool) -> SyncReport {
    if swapped {
        std::mem::swap(&mut report.added_left, &mut report.added_right);
        std::mem::swap(&mut report.updated_left, &mut report.updated_right);
        std::mem::swap(&mut report.deleted_left, &mut report.deleted_right);
        std::mem::swap(&mut report.applied_left, &mut report.applied_right);
    }
    report
}

fn distinct_paths(
    left: &Path,
    right: &Path,
) -> Result<(std::path::PathBuf, std::path::PathBuf), String> {
    use std::os::unix::fs::MetadataExt;
    let left_path =
        std::fs::canonicalize(left).map_err(|error| format!("{}: {error}", left.display()))?;
    let right_path =
        std::fs::canonicalize(right).map_err(|error| format!("{}: {error}", right.display()))?;
    let left_meta =
        std::fs::metadata(&left_path).map_err(|error| format!("{}: {error}", left.display()))?;
    let right_meta =
        std::fs::metadata(&right_path).map_err(|error| format!("{}: {error}", right.display()))?;
    if left_path == right_path
        || (left_meta.dev(), left_meta.ino()) == (right_meta.dev(), right_meta.ino())
    {
        return Err("history sync requires two distinct database files".to_string());
    }
    Ok((left_path, right_path))
}

pub(super) fn checked_events(store: &Store) -> Result<BTreeMap<EventId, HistoryEvent>, String> {
    store.read_checked(|reader| {
        let mut events = BTreeMap::new();
        let mut error = None;
        reader.scan(Tree::SyncEvent, &Span::all(), |key, value| {
            let Some(id) = <&[u8; 32]>::try_from(key).ok().copied().map(EventId) else {
                error = Some("history event key is not 32 bytes".to_string());
                return Walk::Stop;
            };
            let Some(event) = decode_event(id, value) else {
                error = Some(format!("history event {id} is malformed"));
                return Walk::Stop;
            };
            events.insert(id, event);
            Walk::On
        });
        match error {
            Some(error) => Err(error),
            None => Ok(events),
        }
    })
}

pub(super) fn reconcile(
    left: &BTreeMap<EventId, HistoryEvent>,
    right: &BTreeMap<EventId, HistoryEvent>,
) -> (BTreeMap<EventId, HistoryEvent>, SyncReport) {
    let ids: BTreeSet<EventId> = left.keys().chain(right.keys()).copied().collect();
    let mut winners = BTreeMap::new();
    let mut report = SyncReport::default();
    for id in ids {
        match (left.get(&id), right.get(&id)) {
            (Some(left), Some(right)) => {
                let winner = left.preferred(right).clone();
                if left.same_payload(right) {
                    report.unchanged += 1;
                } else {
                    count_change(left, &winner, true, &mut report);
                    count_change(right, &winner, false, &mut report);
                }
                winners.insert(id, winner);
            }
            (Some(event), None) => {
                if event.deleted {
                    report.deleted_right += 1;
                } else {
                    report.added_right += 1;
                }
                winners.insert(id, event.clone());
            }
            (None, Some(event)) => {
                if event.deleted {
                    report.deleted_left += 1;
                } else {
                    report.added_left += 1;
                }
                winners.insert(id, event.clone());
            }
            (None, None) => {}
        }
    }
    (winners, report)
}

fn count_change(
    current: &HistoryEvent,
    winner: &HistoryEvent,
    left: bool,
    report: &mut SyncReport,
) {
    if current == winner {
        report.unchanged += 1;
    } else if winner.deleted && !current.deleted {
        if left {
            report.deleted_left += 1;
        } else {
            report.deleted_right += 1;
        }
    } else if left {
        report.updated_left += 1;
    } else {
        report.updated_right += 1;
    }
}

pub(super) fn write_winners(
    store: &Store,
    winners: &BTreeMap<EventId, HistoryEvent>,
) -> Result<(), String> {
    let all: Vec<&HistoryEvent> = winners.values().collect();
    for chunk in all.chunks(MIGRATION_BATCH) {
        store.write_checked(|writer| {
            for event in chunk {
                let winner = event_of(writer, event.id)
                    .map(|current| current.preferred(event).clone())
                    .unwrap_or_else(|| (*event).clone());
                put_event(writer, &winner)
                    .ok_or_else(|| format!("cannot write history event {}", event.id))?;
            }
            Ok(())
        })?;
    }
    Ok(())
}

fn apply_winners(
    store: &Store,
    winners: &BTreeMap<EventId, HistoryEvent>,
) -> Result<usize, String> {
    let all: Vec<&HistoryEvent> = winners.values().collect();
    let mut applied = 0;
    for chunk in all.chunks(MIGRATION_BATCH) {
        applied += store.write_checked(|writer| {
            let mut changed = 0;
            for event in chunk {
                let event = event_of(writer, event.id)
                    .ok_or_else(|| format!("history event {} disappeared", event.id))?;
                changed += usize::from(
                    apply_event(writer, &event)
                        .ok_or_else(|| format!("cannot project history event {}", event.id))?,
                );
            }
            Ok(changed)
        })?;
    }
    Ok(applied)
}

fn matches_filter(event: &HistoryEvent, filter: &HistoryFilter) -> bool {
    if event.deleted && !filter.include_deleted {
        return false;
    }
    if let Some(query) = filter.query.as_deref() {
        let matches = match filter.matching {
            HistoryMatch::Exact => event.line == query,
            HistoryMatch::Prefix => event.line.starts_with(query),
            HistoryMatch::Contains => event.line.contains(query),
        };
        if !matches {
            return false;
        }
    }
    if filter
        .host
        .as_deref()
        .is_some_and(|host| event.host != host)
    {
        return false;
    }
    if filter
        .cwd
        .as_deref()
        .is_some_and(|cwd| event.completion.as_ref().is_none_or(|done| done.cwd != cwd))
    {
        return false;
    }
    if filter.status.is_some_and(|status| {
        event.completion.as_ref().and_then(|done| done.status) != Some(status)
    }) {
        return false;
    }
    if filter.since.is_some_and(|since| event.recorded_at < since) {
        return false;
    }
    if filter
        .before
        .is_some_and(|before| event.recorded_at >= before)
    {
        return false;
    }
    true
}

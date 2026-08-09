use super::db::{SCHEMA, SCHEMA_VERSION, capped, insert_imported_dir, meta, read_dir, set_meta};
use super::kv::{Fields, Key, Span, Store, Tree, Walk, Writer};
use super::log::Entry;
use super::outcome::Outcome;
use super::row::{DirRow, RunRow, key};
use sha2::{Digest, Sha256};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{Display, Formatter};
use std::path::Path;
use std::str::FromStr;

const EVENT_FORMAT: u64 = 1;
const PROJECTION_FORMAT: u64 = 1;
const NEVER: i64 = i64::MIN;
const MIGRATION_CURSOR: &str = "sync_migration_before";
const MIGRATION_BATCH: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EventId(pub [u8; 32]);

impl EventId {
    fn random() -> Option<EventId> {
        let mut bytes = [0; 32];
        getrandom::fill(&mut bytes).ok()?;
        Some(EventId(bytes))
    }

    fn legacy(key: &[u8], value: &[u8]) -> EventId {
        let mut digest = Sha256::new();
        digest.update((key.len() as u64).to_be_bytes());
        digest.update(key);
        digest.update(value);
        EventId(digest.finalize().into())
    }
}

impl Display for EventId {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl FromStr for EventId {
    type Err = &'static str;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        if text.len() != 64 {
            return Err("event id must contain 64 hexadecimal characters");
        }
        if !text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err("event id must use lowercase hexadecimal characters");
        }
        let mut bytes = [0; 32];
        for (at, byte) in bytes.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&text[at * 2..at * 2 + 2], 16)
                .map_err(|_| "event id contains a non-hexadecimal character")?;
        }
        Ok(EventId(bytes))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistorySegment {
    pub segment: u32,
    pub join: String,
    pub text: String,
    pub status: Option<i32>,
    pub duration_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryCompletion {
    pub cwd: String,
    pub root: Option<String>,
    pub status: Option<i32>,
    pub duration_ms: i64,
    pub segments: Vec<HistorySegment>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryEvent {
    pub id: EventId,
    pub revision: u64,
    pub deleted: bool,
    pub tie_breaker: [u8; 16],
    pub line: String,
    pub mode: String,
    pub recorded_at: u64,
    pub host: String,
    pub session: String,
    pub seq: u32,
    pub rewritten: bool,
    pub completion: Option<HistoryCompletion>,
}

impl HistoryEvent {
    fn local(entry: &Entry, recorded_at: u64) -> Option<HistoryEvent> {
        let id = EventId::random()?;
        Some(HistoryEvent {
            id,
            revision: 1,
            deleted: false,
            tie_breaker: random_tie()?,
            line: entry.line.clone(),
            mode: entry.mode.clone(),
            recorded_at,
            host: super::session::host(),
            session: super::session::id(),
            seq: entry.seq,
            rewritten: entry.rewritten,
            completion: None,
        })
    }

    fn legacy(id: EventId, entry: &Entry, recorded_at: u64) -> HistoryEvent {
        let mut tie_breaker = [0; 16];
        tie_breaker.copy_from_slice(&id.0[..16]);
        HistoryEvent {
            id,
            revision: 1,
            deleted: false,
            tie_breaker,
            line: entry.line.clone(),
            mode: entry.mode.clone(),
            recorded_at,
            host: String::new(),
            session: if entry.session != 0 {
                format!("legacy-{}", entry.session)
            } else {
                String::new()
            },
            seq: entry.seq,
            rewritten: entry.rewritten,
            completion: None,
        }
    }

    pub fn preferred<'a>(&'a self, other: &'a HistoryEvent) -> &'a HistoryEvent {
        match self.stamp().cmp(&other.stamp()) {
            Ordering::Less => other,
            _ => self,
        }
    }

    pub fn same_payload(&self, other: &HistoryEvent) -> bool {
        self.id == other.id
            && self.deleted == other.deleted
            && self.line == other.line
            && self.mode == other.mode
            && self.recorded_at == other.recorded_at
            && self.host == other.host
            && self.session == other.session
            && self.seq == other.seq
            && self.rewritten == other.rewritten
            && self.completion == other.completion
    }

    fn stamp(&self) -> (u64, bool, [u8; 16]) {
        (self.revision, self.deleted, self.tie_breaker)
    }

    fn advance(&mut self) -> Option<()> {
        self.revision = self.revision.checked_add(1)?;
        self.tie_breaker = random_tie()?;
        Some(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HistoryMatch {
    Exact,
    Prefix,
    #[default]
    Contains,
}

#[derive(Debug, Clone, Default)]
pub struct HistoryFilter {
    pub query: Option<String>,
    pub matching: HistoryMatch,
    pub host: Option<String>,
    pub cwd: Option<String>,
    pub status: Option<i32>,
    pub since: Option<u64>,
    pub before: Option<u64>,
    pub include_deleted: bool,
    pub oldest_first: bool,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HistoryStatus {
    pub path: String,
    pub schema: i64,
    pub file_size: u64,
    pub events: usize,
    pub visible: usize,
    pub tombstones: usize,
    pub projections: usize,
    pub pending_projections: usize,
    pub page_size: u64,
    pub allocated_pages: u64,
    pub free_pages: u64,
    pub pending_pages: u64,
    pub active_readers: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncReport {
    pub added_left: usize,
    pub updated_left: usize,
    pub deleted_left: usize,
    pub added_right: usize,
    pub updated_right: usize,
    pub deleted_right: usize,
    pub unchanged: usize,
    pub applied_left: usize,
    pub applied_right: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportReport {
    pub added: usize,
    pub updated: usize,
    pub deleted: usize,
    pub unchanged: usize,
    pub applied: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Contribution {
    dir_id: u64,
    mode: String,
    argv: String,
    head: String,
    at: i64,
    status: Option<i64>,
    duration_ms: i64,
    session: String,
    host: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Projection {
    local_id: u64,
    revision: u64,
    deleted: bool,
    tie_breaker: [u8; 16],
    hidden: bool,
    contribution: Option<Contribution>,
}

impl Projection {
    fn for_local(local_id: u64, event: &HistoryEvent) -> Projection {
        Projection {
            local_id,
            revision: event.revision,
            deleted: event.deleted,
            tie_breaker: event.tie_breaker,
            hidden: false,
            contribution: None,
        }
    }

    fn stamp(&self) -> (u64, bool, [u8; 16]) {
        (self.revision, self.deleted, self.tie_breaker)
    }
}

fn random_tie() -> Option<[u8; 16]> {
    let mut bytes = [0; 16];
    getrandom::fill(&mut bytes).ok()?;
    Some(bytes)
}

fn event_key(id: EventId) -> Vec<u8> {
    id.0.to_vec()
}

fn local_key(id: u64) -> Vec<u8> {
    Key::with_capacity(8).int(id).done()
}

fn encode_event(event: &HistoryEvent) -> Vec<u8> {
    let mut value = Key::with_capacity(event.line.len() + event.mode.len() + 160)
        .int(EVENT_FORMAT)
        .int(event.revision)
        .int(u64::from(event.deleted))
        .blob(&event.tie_breaker)
        .int(event.recorded_at)
        .int(u64::from(event.seq))
        .int(u64::from(event.rewritten))
        .text(&event.line)
        .text(&event.mode)
        .text(&event.host)
        .text(&event.session)
        .int(u64::from(event.completion.is_some()));
    if let Some(done) = &event.completion {
        value = value
            .text(&done.cwd)
            .text(done.root.as_deref().unwrap_or(""))
            .signed(done.status.map_or(NEVER, i64::from))
            .signed(done.duration_ms)
            .int(done.segments.len() as u64);
        for segment in &done.segments {
            value = value
                .int(u64::from(segment.segment))
                .text(&segment.join)
                .text(&segment.text)
                .signed(segment.status.map_or(NEVER, i64::from))
                .signed(segment.duration_ms);
        }
    }
    value.done()
}

fn settle_legacy_tie(event: &mut HistoryEvent) {
    let digest = Sha256::digest(encode_event(event));
    event.tie_breaker.copy_from_slice(&digest[..16]);
}

fn decode_event(id: EventId, value: &[u8]) -> Option<HistoryEvent> {
    let mut fields = Fields::of(value);
    (fields.int()? == EVENT_FORMAT).then_some(())?;
    let revision = fields.int()?;
    (revision != 0).then_some(())?;
    let deleted = match fields.int()? {
        0 => false,
        1 => true,
        _ => return None,
    };
    let tie = fields.blob()?;
    let tie_breaker: [u8; 16] = tie.as_ref().try_into().ok()?;
    let recorded_at = fields.int()?;
    let seq = u32::try_from(fields.int()?).ok()?;
    let rewritten = match fields.int()? {
        0 => false,
        1 => true,
        _ => return None,
    };
    let line = fields.text()?.into_owned();
    let mode = fields.text()?.into_owned();
    let host = fields.text()?.into_owned();
    let session = fields.text()?.into_owned();
    let has_completion = match fields.int()? {
        0 => false,
        1 => true,
        _ => return None,
    };
    let completion = if has_completion {
        let cwd = fields.text()?.into_owned();
        let root = fields.text()?.into_owned();
        let status = optional_status(fields.signed()?)?;
        let duration_ms = fields.signed()?;
        let count = usize::try_from(fields.int()?).ok()?;
        let mut segments = Vec::with_capacity(count.min(1024));
        for _ in 0..count {
            segments.push(HistorySegment {
                segment: u32::try_from(fields.int()?).ok()?,
                join: fields.text()?.into_owned(),
                text: fields.text()?.into_owned(),
                status: optional_status(fields.signed()?)?,
                duration_ms: fields.signed()?,
            });
        }
        Some(HistoryCompletion {
            cwd,
            root: (!root.is_empty()).then_some(root),
            status,
            duration_ms,
            segments,
        })
    } else {
        None
    };
    fields.is_empty().then_some(HistoryEvent {
        id,
        revision,
        deleted,
        tie_breaker,
        line,
        mode,
        recorded_at,
        host,
        session,
        seq,
        rewritten,
        completion,
    })
}

fn optional_status(status: i64) -> Option<Option<i32>> {
    if status == NEVER {
        Some(None)
    } else {
        i32::try_from(status).ok().map(Some)
    }
}

fn encode_projection(projection: &Projection) -> Vec<u8> {
    let mut value = Key::with_capacity(160)
        .int(PROJECTION_FORMAT)
        .int(projection.local_id)
        .int(projection.revision)
        .int(u64::from(projection.deleted))
        .blob(&projection.tie_breaker)
        .int(u64::from(projection.hidden))
        .int(u64::from(projection.contribution.is_some()));
    if let Some(run) = &projection.contribution {
        value = value
            .int(run.dir_id)
            .text(&run.mode)
            .text(&run.argv)
            .text(&run.head)
            .signed(run.at)
            .signed(run.status.unwrap_or(NEVER))
            .signed(run.duration_ms)
            .text(&run.session)
            .text(&run.host);
    }
    value.done()
}

fn decode_projection(value: &[u8]) -> Option<Projection> {
    let mut fields = Fields::of(value);
    (fields.int()? == PROJECTION_FORMAT).then_some(())?;
    let local_id = fields.int()?;
    let revision = fields.int()?;
    (revision != 0).then_some(())?;
    let deleted = match fields.int()? {
        0 => false,
        1 => true,
        _ => return None,
    };
    let tie = fields.blob()?;
    let tie_breaker = tie.as_ref().try_into().ok()?;
    let hidden = match fields.int()? {
        0 => false,
        1 => true,
        _ => return None,
    };
    let has_contribution = match fields.int()? {
        0 => false,
        1 => true,
        _ => return None,
    };
    let contribution = if has_contribution {
        Some(Contribution {
            dir_id: fields.int()?,
            mode: fields.text()?.into_owned(),
            argv: fields.text()?.into_owned(),
            head: fields.text()?.into_owned(),
            at: fields.signed()?,
            status: match fields.signed()? {
                NEVER => None,
                status => Some(status),
            },
            duration_ms: fields.signed()?,
            session: fields.text()?.into_owned(),
            host: fields.text()?.into_owned(),
        })
    } else {
        None
    };
    fields.is_empty().then_some(Projection {
        local_id,
        revision,
        deleted,
        tie_breaker,
        hidden,
        contribution,
    })
}

fn put_event(writer: &Writer<'_, '_>, event: &HistoryEvent) -> Option<()> {
    writer.put(Tree::SyncEvent, event_key(event.id), encode_event(event))
}

fn event_of(writer: &Writer<'_, '_>, id: EventId) -> Option<HistoryEvent> {
    decode_event(id, &writer.get(Tree::SyncEvent, &event_key(id))?)
}

fn id_for_local(writer: &Writer<'_, '_>, local_id: u64) -> Option<EventId> {
    let value = writer.get(Tree::HistoryEvent, &local_key(local_id))?;
    Some(EventId(value.try_into().ok()?))
}

fn projection_of(writer: &Writer<'_, '_>, id: EventId) -> Option<Projection> {
    decode_projection(&writer.get(Tree::EventProjection, &event_key(id))?)
}

fn put_projection(writer: &Writer<'_, '_>, id: EventId, projection: &Projection) -> Option<()> {
    writer.put(
        Tree::EventProjection,
        event_key(id),
        encode_projection(projection),
    )
}

mod admin;
mod projection;

pub use admin::{status_file, sync_files, verify_file};
pub(in crate::track) use projection::{
    append_local, complete_local, hide_local, migrate, rewrite_local, tombstone_local,
};

#[cfg(test)]
use admin::{checked_events, reconcile, write_winners};
#[cfg(test)]
use projection::migrate_row;
#[cfg(test)]
mod tests_lifecycle;
#[cfg(test)]
mod tests_sync;

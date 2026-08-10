//! What you are about to type, and what you meant by what failed.
//!
//! A [`vista::Predictor`] fed from the command history this crate already keeps. Two questions,
//! one model: *what comes next* (a suggestion source) and *what did you mean* (repair, the thing
//! `thefuck` does with two hundred hand-written rules and this does with none).
//!
//! # Why this lives beside the history rather than above it
//!
//! Every field the model wants is already recorded — the line, which shell typed it, where in that
//! shell's run it came, and how it turned out. Nothing new is written to make prediction work; see
//! [`Model::learn`] for the mapping. Putting the model anywhere else would mean carrying that data
//! up a layer to meet it.
//!
//! # What the model is told, and what it is not
//!
//! * **The session is the stream.** Ordering only means something within one shell: the command
//!   you ran here after `cargo build` says nothing about what somebody's other terminal did in
//!   between. `Entry::session` is already exactly this.
//! * **A stream orders candidates; it does not fence them.** Measured, not assumed: a command from
//!   another terminal *is* offered here, because the recent cache is global — and it ranks below
//!   what this shell actually does, by a wide margin once there is any pattern to go on. That is
//!   the right behaviour for a shell (the history source has always offered other shells' lines)
//!   but it is not what "the session is the stream" would lead you to expect, so it is written
//!   down and pinned by a test.
//! * **`seq` skips, and the skip is the point.** A secret command is never appended, so a
//!   per-session counter that jumps from 4 to 6 is the log saying *something happened here that
//!   you cannot see*. Handing that gap to the model unchanged is what stops it learning a
//!   transition that never occurred — see the note on [`crate::track::log::Entry::seq`].
//! * **A secret line is not learned**, because it was never recorded to begin with. That is a
//!   property of the log rather than a rule here, which is the strongest form it could take.

use crate::track::log::Entry;
use vista::{Config, Feature, Item, Observation, Position, Predictor, Query, StreamId};

/// The kind every command is filed under. vista partitions by kind; a shell has one.
const KIND: &str = "command";

/// A predictor, and the history it was built from.
pub struct Model {
    predictor: Predictor,
    /// How many observations went in. `stats()` answers what the model made of them; this answers
    /// what it was shown, which is the number to look at when it has learned nothing.
    learned: usize,
}

impl Model {
    /// An empty model with oslo's bounds.
    pub fn new() -> Model {
        Model {
            predictor: Predictor::new(Self::config()),
            learned: 0,
        }
    }

    /// The bounds this shell runs the model under.
    ///
    /// vista's defaults, deliberately and for now: every one of its twenty-five limits is already
    /// a bound rather than an invitation, and tuning them before there is a measurement to tune
    /// against would be choosing numbers by taste. The one thing this does fix is the shape of the
    /// answer — a shell asks for a handful of candidates, never a page.
    fn config() -> Config {
        Config::default()
    }

    /// Learn one recorded command.
    ///
    /// Returns whether it was taken. A line with no session — written before sessions were
    /// recorded — is skipped rather than filed under a shared stream 0, which would teach the
    /// model transitions between unrelated shells.
    pub fn learn(&mut self, entry: &Entry, at: i64) -> bool {
        self.learn_outcome(entry, at, None)
    }

    /// Learn one recorded command, saying whether it worked.
    ///
    /// **`ok` is what makes repair possible at all.** vista learns a correction from a failure
    /// followed by a retyping — it pairs an observation marked failed with the next one in the same
    /// stream — so a shell that reports no outcome teaches it no corrections, however much history
    /// it replays. The name `success` is not oslo's choice: it is one of the outcome names vista
    /// reads as a quality, and spelling it anything else would make the feature inert but silent.
    ///
    /// `None` is neither success nor failure, and is what a replayed log row gets: the old rows
    /// predate outcomes, and inventing one would teach the model a fact nobody recorded.
    pub fn learn_outcome(&mut self, entry: &Entry, at: i64, ok: Option<bool>) -> bool {
        if entry.session == 0 || entry.seq == 0 || entry.line.trim().is_empty() {
            return false;
        }
        let observation = Observation {
            item: Item::new(KIND, entry.line.clone()),
            stream: StreamId(u64::from(entry.session)),
            position: Position(u64::from(entry.seq)),
            timestamp: at,
            context: vec![Feature::categorical("mode", entry.mode.clone())],
            outcome: ok
                .map(|ok| {
                    vec![Feature::categorical(
                        "success",
                        if ok { "true" } else { "false" },
                    )]
                })
                .unwrap_or_default(),
        };
        let taken = self.predictor.observe(observation).is_ok();
        if taken {
            self.learned += 1;
        }
        taken
    }

    /// Learn a run of recorded commands, oldest first.
    ///
    /// **Oldest first, and that is not a detail.** The model is a sequence model: fed backwards it
    /// would learn that `cargo build` follows `cargo test`. `Log::recent` answers newest-first,
    /// so whoever calls this reverses — and the test below is what keeps that true.
    pub fn learn_all<'a>(&mut self, entries: impl IntoIterator<Item = &'a Entry>) -> usize {
        let mut taken = 0;
        for (at, entry) in entries.into_iter().enumerate() {
            if self.learn(entry, at as i64) {
                taken += 1;
            }
        }
        taken
    }

    /// What this shell is likely to run next.
    ///
    /// `partial` is what has been typed so far, when anything has. `limit` is small by nature:
    /// this answers a ghost suggestion, and the ghost shows one.
    pub fn next(&self, session: u32, seq: u32, partial: Option<&str>, limit: usize) -> Vec<Guess> {
        let mut query = Query::new(
            StreamId(u64::from(session)),
            Position(u64::from(seq)),
            limit,
        );
        query.partial = partial.map(str::to_string);
        self.predictor
            .predict(&query)
            .into_iter()
            .map(Guess::from)
            .collect()
    }

    /// What a failed line was probably meant to be.
    ///
    /// Rebuilt from commands history already holds, so it can only ever propose something that has
    /// really been run — which is the safety property a rule engine cannot offer.
    pub fn repair(&self, session: u32, seq: u32, failed: &str, limit: usize) -> Vec<Guess> {
        let query = Query::new(
            StreamId(u64::from(session)),
            Position(u64::from(seq)),
            limit,
        );
        self.predictor
            .predict_aligned(&query, &Item::new(KIND, failed))
            .into_iter()
            .map(Guess::from)
            .filter(|guess| guess.line != failed)
            .collect()
    }

    /// How many observations were taken.
    pub fn learned(&self) -> usize {
        self.learned
    }

    /// The retypings this model has watched: what was typed, what was run instead, how often.
    ///
    /// The evidence behind a repair, rather than the repair itself. Exposed because "the model
    /// learned nothing to correct from" and "the model had nothing to say about this line" look
    /// identical from `repair` alone, and only one of them is a bug.
    pub fn corrections(&self) -> Vec<(String, String, u64)> {
        self.predictor
            .corrections()
            .into_iter()
            .map(|(pair, count)| (pair.typed.value, pair.corrected.value, count))
            .collect()
    }

    /// Write the model where it can be read back without replaying history.
    pub fn save(&self, writer: impl std::io::Write) -> Result<(), vista::SnapshotError> {
        self.predictor.write_snapshot(writer)
    }

    /// Read one back.
    ///
    /// **The same three components `Model::new` builds with.** A snapshot records which normalizer,
    /// tokenizer and matcher wrote it and refuses any other set, which is the right thing to do —
    /// a model read under a different matcher would rank by rules it was not built under. Naming
    /// them here twice is the cost of that check; getting one wrong answers `IncompatibleConfig`
    /// and no model at all.
    pub fn load(reader: impl std::io::Read) -> Result<Model, vista::SnapshotError> {
        let predictor = Predictor::read_snapshot(
            Self::config(),
            vista::IdentityNormalizer,
            vista::WhitespaceTokenizer,
            vista::ContainsMatcher,
            reader,
        )?;
        // The count is of observations *this process* made. A model read back from disk has
        // learned plenty and been shown nothing, and saying "0" is more honest than inventing a
        // number the snapshot does not carry.
        Ok(Model {
            predictor,
            learned: 0,
        })
    }

    /// Write the snapshot to `path`, readable only by its owner.
    ///
    /// **Written beside, then renamed.** A snapshot half-written over the live one is worse than
    /// no snapshot at all: it would be read at the next start and fail, and the failure would look
    /// like a bug in the model rather than a torn file. The `0600` is set on the temporary before
    /// the rename, so the finished file is never briefly world-readable — it holds every command
    /// you have run, which makes it exactly as sensitive as the history it came from.
    pub fn save_to(&self, path: &std::path::Path) -> std::io::Result<()> {
        use std::os::unix::fs::PermissionsExt;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let scratch = path.with_extension("model.new");
        {
            let file = std::fs::File::create(&scratch)?;
            file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
            self.save(&file)
                .map_err(|e| std::io::Error::other(format!("{e:?}")))?;
            file.sync_all()?;
        }
        std::fs::rename(&scratch, path)
    }

    /// Read the snapshot at `path`, or `None` when there is not a usable one.
    ///
    /// Every failure is the same answer — a missing file, a truncated one, a snapshot from an
    /// older vista. None of them is worth a message: the model is a cache of history and the only
    /// cost of losing it is learning again.
    pub fn load_from(path: &std::path::Path) -> Option<Model> {
        let file = std::fs::File::open(path).ok()?;
        Model::load(std::io::BufReader::new(file)).ok()
    }
}

/// Where the model is kept, beside the history it was learned from.
pub fn default_path(xdg_data: Option<&str>, home: Option<&str>) -> Option<std::path::PathBuf> {
    crate::track::profile::store_path(xdg_data, home, "model")
}

/// Forget the model on disk, for whatever forgets the history.
///
/// **`oslo history clear` must reach this.** A model is a distillation of every command that has
/// been run; leaving it behind would mean the shell keeps precisely what it was asked to forget,
/// in a file the user does not know exists.
pub fn forget_saved(path: &std::path::Path) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

impl Default for Model {
    fn default() -> Self {
        Self::new()
    }
}

/// The one model this process consults, once something has loaded it.
///
/// A slot rather than a value because there may be no model at all: a shell with no history, a
/// snapshot that could not be read, or a start where loading has not finished yet. Every reader
/// treats "no model" as "no suggestion", which is the same answer the other suggestion sources
/// give when they have nothing.
fn shared() -> &'static std::sync::RwLock<Option<Model>> {
    static MODEL: std::sync::OnceLock<std::sync::RwLock<Option<Model>>> =
        std::sync::OnceLock::new();
    MODEL.get_or_init(|| std::sync::RwLock::new(None))
}

/// Load the model from `path`, on a thread of its own.
///
/// **Loaded, never replayed, and never on the startup path.** Measured on this machine
/// (`bench/predict.rs`): reading a snapshot costs 0.1 ms and the file is ~31 KB whatever the
/// history size, because `Config` bounds it. *Building* the same model from history costs 9.5 ms
/// at ten thousand commands and 47 ms at fifty thousand — oslo starts in about 3.5 ms, so replay
/// is not merely slower than the snapshot, it is several times the whole startup.
///
/// Detached, like [`crate::track`]'s own warming and the `$PATH` scan: nothing waits for it, and a
/// prompt drawn before it lands simply has no prediction to offer yet.
pub fn warm(path: std::path::PathBuf) {
    std::thread::spawn(move || {
        let Some(model) = Model::load_from(&path) else {
            return;
        };
        if let Ok(mut slot) = shared().write() {
            // Only if nothing has been learned in the meantime. A command run while the snapshot
            // was still being read is worth more than the snapshot's view of the world.
            if slot.is_none() {
                *slot = Some(model);
            }
        }
    });
}

/// Where this shell is: which session, and how many commands into it.
///
/// Kept here rather than passed down because the ghost suggestion is asked for by the line editor,
/// which knows what has been typed and nothing about sessions. Threading two numbers through
/// `suggest(line, pos)` and everything that calls it would spread the model across three crates to
/// save one atomic.
static AT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Say where the next command will fall. Called once per prompt.
pub fn note_position(session: u32, seq: u32) {
    AT.store(
        (u64::from(session) << 32) | u64::from(seq),
        std::sync::atomic::Ordering::Relaxed,
    );
}

/// Session and position as last noted.
fn position() -> (u32, u32) {
    let packed = AT.load(std::sync::atomic::Ordering::Relaxed);
    ((packed >> 32) as u32, packed as u32)
}

/// What comes next here, for whoever is drawing a suggestion.
///
/// **A shell that has run nothing yet still gets an answer.** Before the first command there is no
/// session to ask about — the log allocates an ordinal lazily, on purpose, so that opening a shell
/// and closing it costs no write — and asking under stream 0 finds no per-stream history and falls
/// back to what the model knows globally. That is exactly the right answer for a first command: it
/// is what the recent cache is for, and it is the only thing a fresh shell could be told.
///
/// Returning nothing instead was the first attempt, and it meant prediction was silent at every
/// prompt until you had already typed something — which is most of the prompts where it would have
/// been useful.
pub fn suggest_here(partial: Option<&str>, limit: usize) -> Vec<Guess> {
    let (session, seq) = position();
    suggest(session, seq, partial, limit)
}

/// What a failed line meant, at the position last noted.
pub fn repair_here(failed: &str, limit: usize) -> Vec<Guess> {
    let (session, seq) = position();
    repair(session, seq, failed, limit)
}

/// Ask the shared model what comes next. Empty when there is no model.
pub fn suggest(session: u32, seq: u32, partial: Option<&str>, limit: usize) -> Vec<Guess> {
    shared()
        .read()
        .ok()
        .and_then(|slot| slot.as_ref().map(|m| m.next(session, seq, partial, limit)))
        .unwrap_or_default()
}

/// Ask the shared model what a failed line meant. Empty when there is no model.
pub fn repair(session: u32, seq: u32, failed: &str, limit: usize) -> Vec<Guess> {
    shared()
        .read()
        .ok()
        .and_then(|slot| slot.as_ref().map(|m| m.repair(session, seq, failed, limit)))
        .unwrap_or_default()
}

/// The line that has been logged but has not finished, waiting for its status.
static HELD: std::sync::RwLock<Option<(Entry, i64)>> = std::sync::RwLock::new(None);

/// Take a command the log has just written, and wait before learning it.
///
/// **The wait is the point.** A line is logged *before* it runs, so that a long command is visible
/// to another terminal while it is still going — which means its status does not exist yet, and
/// vista fixes an observation's outcome when it takes it. Learning here would file every command as
/// neither working nor failing, and the correction pairs repair is built on would never form.
///
/// So the observation is held until [`settle`] has the status. What is not deferred is the
/// position: the next prompt asks from it, and it is known now.
pub fn record(entry: &Entry, at: i64) {
    note_position(entry.session, entry.seq.saturating_add(1));
    if let Ok(mut held) = HELD.write() {
        // A previous line still held means its command boundary never came — a shell killed
        // mid-command, or a path that logs without running. Learn it outcome-less rather than
        // drop it: the sequence matters more than the one fact nobody recorded.
        if let Some((entry, at)) = held.take() {
            learn(&entry, at, None);
        }
        *held = Some((entry.clone(), at));
    }
}

/// Learn the held line, now that it is known whether it worked.
///
/// `status` is what the command boundary reports: `None` for a line that never reached execution,
/// which is neither outcome. Called once per command; a second call with nothing held does nothing.
pub fn settle(status: Option<i32>) {
    let held = HELD.write().ok().and_then(|mut held| held.take());
    if let Some((entry, at)) = held
        && ran(status)
    {
        learn(&entry, at, status.map(|status| status == 0));
    }
}

/// Whether the line got as far as being a command at all.
///
/// **A line that never ran is not learned, and this is what keeps repair working.** Measured: once
/// a mistyped line is in the model as something that has been run, repair for that exact line
/// answers nothing — it is a command like any other, so there is nothing to align it *to*. Which is
/// the case that matters, because you ask for a repair immediately after the failure that prompted
/// it. Dropping these also stops a typo being offered as a suggestion for ever, which the
/// tracker's `RunRow::worth_suggesting` already refuses to do for the same reason.
///
/// **Not "it failed".** `cargo build` that failed to compile and `git push` that was rejected are
/// among the most predictive lines a shell sees, and they are learned — marked failed, which is
/// what the outcome weight and vista's correction pairs are for. The line drawn here is narrower:
/// 127 is no such command, 126 is not executable, and `None` never reached execution. A real
/// program can exit 127, and the cost of that is one line not learned.
fn ran(status: Option<i32>) -> bool {
    !matches!(status, None | Some(126) | Some(127))
}

/// Teach the shared model, creating it if the snapshot has not arrived — a shell with no history
/// still learns from the session it is having, which is the only history a first run has.
fn learn(entry: &Entry, at: i64, ok: Option<bool>) {
    if let Ok(mut slot) = shared().write() {
        slot.get_or_insert_with(Model::new)
            .learn_outcome(entry, at, ok);
    }
}

/// Write the shared model to `path`, if there is one. Answers whether anything was written.
pub fn save_shared(path: &std::path::Path) -> bool {
    shared()
        .read()
        .ok()
        .and_then(|slot| slot.as_ref().map(|m| m.save_to(path).is_ok()))
        .unwrap_or(false)
}

/// Whether there is a model to ask.
///
/// Distinct from "the answer was empty", which is also what a model with nothing matching says.
/// A shell that has just started may not have read its snapshot yet.
pub fn ready() -> bool {
    shared().read().is_ok_and(|slot| slot.is_some())
}

/// Drop the model this process is holding, for whatever drops the history.
pub fn forget_shared() {
    if let Ok(mut slot) = shared().write() {
        *slot = None;
    }
}

/// One answer, with what the model thought of it.
#[derive(Debug, Clone, PartialEq)]
pub struct Guess {
    pub line: String,
    pub probability: f64,
}

impl From<vista::Prediction> for Guess {
    fn from(prediction: vista::Prediction) -> Guess {
        Guess {
            line: prediction.item.value.to_string(),
            probability: prediction.probability,
        }
    }
}

#[cfg(test)]
mod tests;

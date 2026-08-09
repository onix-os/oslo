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
        if entry.session == 0 || entry.seq == 0 || entry.line.trim().is_empty() {
            return false;
        }
        let observation = Observation {
            item: Item::new(KIND, entry.line.clone()),
            stream: StreamId(u64::from(entry.session)),
            position: Position(u64::from(entry.seq)),
            timestamp: at,
            context: vec![Feature::categorical("mode", entry.mode.clone())],
            outcome: Vec::new(),
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

    /// Write the model where it can be read back without replaying history.
    pub fn save(&self, writer: impl std::io::Write) -> Result<(), vista::SnapshotError> {
        self.predictor.write_snapshot(writer)
    }
}

impl Default for Model {
    fn default() -> Self {
        Self::new()
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

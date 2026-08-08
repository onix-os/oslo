//! Persistence and shared access for the frecency table.
//!
//! [`crate::spec::FrecencyTracker`] had no callers at all: `record_use` was never
//! invoked, so `get_score` answered 0.0 for everything and the completion sort collapsed to
//! alphabetical — typing `exit` offered `exitsnoop-bpfcc`. This wraps the tracker in the two
//! things that were missing: interior mutability, because the completer only ever holds `&self`,
//! and a file, because a ranking that resets every session never gets good.
//!
//! The file is an append-only log of `count<TAB>timestamp<TAB>name` lines, folded on load. An
//! append is atomic enough for the small records involved, so two shells running side by side
//! both keep their uses instead of the last one out overwriting the other — which is exactly the
//! failure the history file has.

use super::spec::FrecencyTracker;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// Rewrite the log once it has this many lines, so a long-lived shell does not grow it forever.
const COMPACT_AT: usize = 4096;

pub struct FrecencyStore {
    tracker: Mutex<FrecencyTracker>,
    path: Option<PathBuf>,
}

impl FrecencyStore {
    /// Load the table from `path`, if it is readable.
    ///
    /// A missing or corrupt file is not an error: ranking is an optimisation, and refusing to
    /// start a shell over it would be absurd.
    pub fn load(path: Option<PathBuf>) -> Self {
        let mut tracker = FrecencyTracker::new();
        let mut lines = 0usize;
        if let Some(ref p) = path
            && let Ok(text) = fs::read_to_string(p)
        {
            for line in text.lines() {
                lines += 1;
                if let Some((count, last, name)) = parse_line(line) {
                    tracker.merge(name, count, last);
                }
            }
        }

        let store = Self {
            tracker: Mutex::new(tracker),
            path,
        };
        if lines >= COMPACT_AT {
            store.compact();
        }
        store
    }

    /// A store that never touches the disk. Used by tests and by non-interactive shells.
    pub fn in_memory() -> Self {
        Self {
            tracker: Mutex::new(FrecencyTracker::new()),
            path: None,
        }
    }

    /// The default location, beside the history file.
    pub fn default_path() -> Option<PathBuf> {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".oslo_frecency"))
    }

    /// Count one use of `name`, in memory and on disk.
    pub fn record(&self, name: &str) {
        if name.is_empty() {
            return;
        }
        let now = now_secs();
        self.tracker.lock().unwrap().record_use(name);
        if let Some(ref p) = self.path {
            append(p, name, now);
        }
    }

    /// This command's rank, higher being better. Zero for anything never used.
    pub fn score(&self, name: &str) -> f64 {
        self.tracker.lock().unwrap().get_score(name)
    }

    /// Rewrite the log as one line per command.
    fn compact(&self) {
        let Some(ref p) = self.path else {
            return;
        };
        let guard = self.tracker.lock().unwrap();
        let mut out = String::new();
        for (name, count, last) in guard.entries() {
            if name.contains(['\t', '\n']) {
                continue;
            }
            out.push_str(&format!("{}\t{}\t{}\n", count, last, name));
        }
        // Written through a sibling temp file so a crash mid-write cannot truncate the table.
        let tmp = p.with_extension("tmp");
        if fs::write(&tmp, out).is_ok() {
            let _ = fs::rename(&tmp, p);
        }
    }
}

fn append(path: &Path, name: &str, now: u64) {
    // A name with a tab or newline in it would corrupt the log; such a command name is not worth
    // ranking.
    if name.contains(['\t', '\n']) {
        return;
    }
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "1\t{}\t{}", now, name);
    }
}

fn parse_line(line: &str) -> Option<(u64, u64, &str)> {
    let mut parts = line.splitn(3, '\t');
    let count = parts.next()?.parse().ok()?;
    let last = parts.next()?.parse().ok()?;
    let name = parts.next()?;
    if name.is_empty() {
        return None;
    }
    Some((count, last, name))
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unused_command_scores_zero() {
        let store = FrecencyStore::in_memory();
        assert_eq!(store.score("git"), 0.0);
    }

    #[test]
    fn use_raises_the_score_and_more_use_raises_it_further() {
        let store = FrecencyStore::in_memory();
        store.record("git");
        let once = store.score("git");
        assert!(once > 0.0);
        store.record("git");
        assert!(store.score("git") > once);
        assert!(store.score("git") > store.score("gcc"));
    }

    #[test]
    fn uses_survive_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("frecency");

        let first = FrecencyStore::load(Some(path.clone()));
        first.record("cargo");
        first.record("cargo");
        first.record("ls");
        let expected = first.score("cargo");
        drop(first);

        let second = FrecencyStore::load(Some(path));
        assert!((second.score("cargo") - expected).abs() < 1e-6);
        assert!(second.score("cargo") > second.score("ls"));
    }

    #[test]
    fn two_shells_do_not_clobber_each_other() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("frecency");

        let a = FrecencyStore::load(Some(path.clone()));
        let b = FrecencyStore::load(Some(path.clone()));
        a.record("apples");
        b.record("bananas");
        drop((a, b));

        let third = FrecencyStore::load(Some(path));
        assert!(third.score("apples") > 0.0);
        assert!(third.score("bananas") > 0.0);
    }

    #[test]
    fn a_corrupt_log_is_ignored_not_fatal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("frecency");
        fs::write(&path, "garbage\n\n1\tnot-a-number\tx\n1\t100\tgood\n").unwrap();

        let store = FrecencyStore::load(Some(path));
        assert!(store.score("good") > 0.0);
        assert_eq!(store.score("x"), 0.0);
    }

    #[test]
    fn a_long_log_is_compacted_on_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("frecency");
        let mut text = String::new();
        for i in 0..COMPACT_AT + 10 {
            text.push_str(&format!("1\t100\tcmd{}\n", i % 5));
        }
        fs::write(&path, text).unwrap();

        let store = FrecencyStore::load(Some(path.clone()));
        assert!(store.score("cmd0") > 0.0);
        let compacted = fs::read_to_string(&path).unwrap();
        assert_eq!(compacted.lines().count(), 5);
    }
}

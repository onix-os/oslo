//! Frequency-and-recency scoring for completion ranking.
//!
//! Ranks a candidate by how often it has been used, decayed by how long ago. Something used a
//! lot but not lately falls behind something used recently.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

pub struct FrecencyTracker {
    // Stores (count, last_timestamp)
    history: HashMap<String, (u64, u64)>,
}

impl Default for FrecencyTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl FrecencyTracker {
    pub fn new() -> Self {
        Self {
            history: HashMap::new(),
        }
    }

    pub fn record_use(&mut self, cmd: &str) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let entry = self.history.entry(cmd.to_string()).or_insert((0, now));
        entry.0 += 1;
        entry.1 = now;
    }

    /// Fold in a use that happened in an earlier session.
    ///
    /// Separate from [`Self::record_use`] because a reloaded entry must keep its original
    /// timestamp: replaying the log through `record_use` would date every past use to startup and
    /// flatten the recency half of the score.
    pub fn merge(&mut self, cmd: &str, count: u64, last_used: u64) {
        let entry = self
            .history
            .entry(cmd.to_string())
            .or_insert((0, last_used));
        entry.0 = entry.0.saturating_add(count);
        entry.1 = entry.1.max(last_used);
    }

    /// Every recorded command with its use count and last-use timestamp, for persistence.
    pub fn entries(&self) -> impl Iterator<Item = (&str, u64, u64)> {
        self.history
            .iter()
            .map(|(name, &(count, last))| (name.as_str(), count, last))
    }

    pub fn get_score(&self, cmd: &str) -> f64 {
        if let Some(&(count, last_time)) = self.history.get(cmd) {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let age_secs = now.saturating_sub(last_time) as f64;
            // Frecency formula: count / (1.0 + ln(1.0 + age_hours))
            let age_hours = age_secs / 3600.0;
            (count as f64) / (1.0 + (1.0 + age_hours).ln())
        } else {
            0.0
        }
    }
}

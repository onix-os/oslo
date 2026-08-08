//! What the command that just ran left behind: `$LINENO`, `PIPESTATUS`, and the status a command
//! substitution exited with.
//!
//! Separate from the variable table these end up in because they are the shell writing down its
//! own state rather than a script assigning to a name — none of them go through the readonly
//! check, and two of them are published only when the number actually moved.

use super::*;

impl Environment {
    /// Publish `line` as `$LINENO`, unless it is already what the variable says.
    ///
    /// The write itself is not free — a `String` for the number, the readonly check, the
    /// representability scan, an array probe and two more `String`s — and it happened before every
    /// command in every list. Interactively that is the same line number every time, because each
    /// typed command is line one; in a loop it is the same handful over and over.
    ///
    /// `published_line` is zeroed by any *other* route to the variable — an assignment or an
    /// `unset` — so the next command republishes rather than trusting a stale record.
    pub fn note_line(&mut self, line: u32) {
        if line == self.published_line {
            return;
        }
        self.set_var("LINENO", &line.to_string(), false);
        self.published_line = line;
    }

    /// Record every stage of a pipeline's exit status, left to right. A one-command pipeline
    /// records a single status, as bash's `PIPESTATUS` does.
    ///
    /// Publishing `PIPESTATUS` here rather than synthesising it during expansion keeps one copy
    /// of the numbers: `${PIPESTATUS[0]}` and `set -o pipefail` cannot disagree about what the
    /// last pipeline did. Written directly into the array table, bypassing the readonly check —
    /// this is the shell recording its own state, not a user assignment.
    pub fn set_pipeline_status(&mut self, statuses: Vec<i32>) {
        // The array is rebuilt only when the numbers moved. Publishing it unconditionally meant a
        // `Vec`, a `"PIPESTATUS"` `String`, a `BTreeMap` and a `String` per stage after every
        // command — and a run of successful commands reports the same `[0]` every time.
        if self.pipeline_status == statuses && self.arrays.contains_key("PIPESTATUS") {
            return;
        }
        self.arrays.insert(
            "PIPESTATUS".to_string(),
            ShellArray::from_values(statuses.iter().map(i32::to_string)),
        );
        self.pipeline_status = statuses;
    }

    /// The stage statuses of the most recent pipeline. Never empty.
    pub fn pipeline_status(&self) -> &[i32] {
        &self.pipeline_status
    }

    /// Record what a command substitution exited with. Called by `exec::substitution`.
    pub fn note_substitution_status(&mut self, status: i32) {
        self.substitution_status = Some(status);
    }

    /// The status of the last command substitution, without consuming it.
    ///
    /// Distinct from [`Environment::take_substitution_status`] because two callers want the same
    /// number for different reasons and must not steal it from each other: an assignment-only
    /// command *reports* it and is done with it, while word expansion only needs to know it in
    /// order to update `$?` mid-word.
    pub fn peek_substitution_status(&self) -> Option<i32> {
        self.substitution_status
    }

    /// Take the status of the last command substitution, clearing it.
    ///
    /// What an assignment-only command reports (POSIX: `x=$(exit 5)` leaves `$?` at 5). Consumed
    /// rather than read, so a later assignment with no substitution reports 0, not a stale number.
    pub fn take_substitution_status(&mut self) -> Option<i32> {
        self.substitution_status.take()
    }
}

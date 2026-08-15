//! Everything that gets written down once a command has finished.
//!
//! One place, because the four writes a command produces are one decision taken four ways: a
//! `pre-record` rule is asked once, and its answer decides what the aggregate learns, what the log
//! row ends up saying, and whether the outcome can ride along with the boundary or has to follow it.
//! Split across the loop, those would be four call sites each re-deriving the same answer.

use super::*;

/// One finished command, as the loop already knows it.
pub(in crate::startup) struct Finished<'a> {
    pub text: &'a str,
    /// Where it started, which is what it is attributed to.
    pub before: &'a str,
    /// Where it left the shell, which is what makes it an arrival.
    pub after: &'a str,
    pub mode: &'a str,
    pub result: &'a Result<i32, ShellError>,
    pub elapsed: Duration,
    /// The log row it went in under, if it was logged at all — which the append has very likely
    /// not answered yet. See [`track::writer::Ticket`].
    pub logged_as: Option<&'a track::writer::Ticket>,
}

impl Tracker {
    /// Record a finished command: the boundary, the aggregate, and what the line did.
    ///
    /// Beside the `postcmd` hook rather than through it — see this module's parent. Every argument
    /// is a local the loop already had and used to drop.
    pub(in crate::startup) fn write_down(&mut self, done: &Finished<'_>) {
        // Asked once, before anything is written down. A rule may keep one link of a chain instead
        // of the whole line, or refuse it.
        let decided =
            ask_what_to_record(done.text, done.before, done.mode, done.result, done.elapsed);
        let lines = lines_to_record(&decided, done.text);
        // What the line did, joined to the log row it went in under.
        //
        // **Handed to the boundary rather than written after it**, because the boundary is what
        // resolves the directory these rows name: two writes asking the same question are one
        // write. Only when nothing is going to rewrite the log row underneath them — a rule that
        // refuses or replaces the line does that *after* the loop, and then the outcome has to wait
        // its turn as it always did.
        let rows = done
            .logged_as
            .filter(|_| !lines.is_empty())
            .map(|row| (row.clone(), outcome_rows(done.result, done.elapsed)));
        let rides_along = rows.is_some() && matches!(decided, Recording::AsTyped);

        // The predictor's half of the same pairing, and it has to travel with them. `record` runs
        // inside the append, on the writer thread; `settle` takes what that held. Left on this
        // thread the two would invert the first time the queue was even slightly behind, and the
        // model would learn a line's status against the line before it.
        if rows.is_some() {
            let status = outcome_status(done.result);
            track::writer::defer(move || settle_prediction(status));
        }

        for (first, line) in lines.iter().enumerate().map(|(i, l)| (i == 0, l)) {
            let run = ran(line, done.mode, done.result, done.elapsed);
            // Only the first arrival records the *movement* and the dwell; a second one would
            // credit this directory twice for the same command.
            if first {
                let carried = rows
                    .as_ref()
                    .filter(|_| rides_along)
                    .map(|(row, rows)| (row.clone(), rows.clone()));
                self.boundary(done.before, done.after, run, carried);
            } else {
                self.also_ran(done.before, run);
            }
        }
        if lines.is_empty() {
            self.forget_boundary();
        }

        // Behind the boundary in the same queue, so they still see the directory it resolved and
        // still land after the log row they rewrite.
        if let Some(row) = done.logged_as.cloned() {
            let (decided, text) = (decided.clone(), done.text.to_string());
            track::writer::defer(move || {
                if let Some(id) = row.id() {
                    settle_log_row(id, &decided, &text);
                }
            });
        }
        if let Some((row, rows)) = rows.filter(|_| !rides_along) {
            track::writer::defer(move || {
                if let Some(id) = row.id() {
                    record_outcome(id, &rows);
                }
            });
        }
    }
}

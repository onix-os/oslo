//! What a `pre-cmd` handler answered, read once.
//!
//! The hook had two answers and now has three, and the third is a table — so the reading is worth
//! its own place rather than growing a `match` in the middle of the loop.
//!
//! | answered | means |
//! |---|---|
//! | nothing | run the line as typed |
//! | a string | run this instead |
//! | `false` | do not run it at all |
//! | `{ text = … }` | run this instead |
//! | `{ record = false }` | run it, and write nothing down |
//! | `{ cancel = true }` | do not run it at all |
//!
//! **The table is additive, and every field is optional.** A handler that returns `{}` has said
//! nothing, which must mean the same as returning nothing at all: the fields say what to *change*,
//! and an absent field changes nothing. That is what lets a second handler be added to a config
//! without the first one's answer suddenly meaning more than it did.

use super::{History, Mode, remember, remember_history};
use oslo_lua::Value;

/// Write the line down, unless it is hidden. Answers the log row, for joining what it *did* to it.
///
/// **Called after `pre-cmd`, and that ordering is the whole of the veto.** These used to run before
/// the hook fired, which was harmless while the only way to hide a line was the leading space — that
/// is known before anything runs. A handler cannot be asked afterwards whether to record something
/// already recorded, and the test that found this watched the line land in `.oslo_history` anyway.
///
/// `entered` is the line as *typed*, not the replacement a handler may have returned: history is
/// what you wrote rather than what ran.
///
/// # Why the append stays on this thread when everything after it does not
///
/// Because this is the row that says the command was typed at all, and it is the only one written
/// *before* the command runs. A shell killed mid-command — the case
/// `docs/features/what-gets-written-down.md` promises about — has this row and nothing else, which
/// is exactly the guarantee worth having.
///
/// Deferring it bought 0.63 ms of prompt-thread CPU and no measurable latency at all: Enter to
/// prompt-ready measured 0.40 ms either way over 150 samples. It cost the guarantee, and it lost
/// the tail of every session ending in `exec`. Everything *after* the command — the boundary, the
/// outcome, a rewrite of this row, the trim — is a record of something already finished, and does
/// go to [`oslo_base::track::writer`].
pub(super) fn write_down(
    history: &mut History,
    entered: &str,
    mode: Mode,
    hidden: bool,
    max_size: usize,
) -> Option<u64> {
    remember(history, entered, hidden);
    if hidden {
        return None;
    }
    // Kept alongside the editor's own copy so a later language switch, which refills that copy from
    // scratch, still finds this line.
    remember_history(entered, mode);
    let db = oslo_base::track::store()?;
    let id = db.append(
        entered,
        match mode {
            Mode::Lua => oslo_base::track::log::MODE_LUA,
            Mode::Shell => oslo_base::track::log::MODE_SHELL,
        },
    );
    // `$HISTSIZE` bounds the log as well as the editor's copy, or the file grows without limit while
    // the shell politely forgets. Amortised, because the trim is a full scan and this used to be one
    // per line typed. **Queued**, unlike the append: nothing waits on the bound being enforced this
    // instant, and the scan is long enough to be felt — see `PLAN.md`.
    oslo_base::track::writer::defer(move || {
        if let Some(db) = oslo_base::track::store() {
            db.trim_soon(max_size.max(1));
        }
    });
    id
}

/// What the loop should do with the line.
pub(super) struct Answer {
    /// The line to run — the original unless a handler replaced it.
    pub text: String,
    /// Whether anything may be written down about it.
    pub record: bool,
}

/// Read the answer, or `None` when the line was cancelled.
pub(super) fn read(answered: Option<Value>, text: String) -> Option<Answer> {
    match answered {
        // `false` is the long-standing spelling of "do not run this".
        Some(Value::Bool(false)) => None,
        Some(Value::Str(replacement)) => Some(Answer {
            text: replacement.to_string(),
            record: true,
        }),
        Some(Value::Table(table)) => {
            let table = table.borrow();
            if table.get(&Value::str("cancel")).truthy() {
                return None;
            }
            Some(Answer {
                text: match table.get(&Value::str("text")) {
                    Value::Str(replacement) => replacement.to_string(),
                    _ => text,
                },
                // **Absent means yes.** Only an explicit `record = false` suppresses; `record = nil`
                // is a handler that did not mention recording, and reading that as a veto would
                // make every table-returning handler hide the line by accident.
                record: !matches!(table.get(&Value::str("record")), Value::Bool(false)),
            })
        }
        _ => Some(Answer { text, record: true }),
    }
}

#[cfg(test)]
#[path = "precmd/tests.rs"]
mod tests;

//! Answers that have not arrived yet, and the editor's willingness to wait for them.
//!
//! # The problem this solves
//!
//! The editor waits for a keystroke, and a blocking wait cannot notice anything else finishing. So
//! anything computed *off* the editor's thread — an asynchronous prompt, and soon a suggestion from
//! a plugin that had to ask something slow — lands in a cache nobody looks at until the next key is
//! pressed. For the last prompt of a session, that is never.
//!
//! Two numbers fix it, and they are the whole of this module:
//!
//! | | |
//! |---|---|
//! | [`outstanding`] | is an answer still coming? While true, the input wait comes up for air |
//! | [`generation`] | has anything landed? A changed value means the frame is stale |
//!
//! The editor's `next_input` blocks when nothing is outstanding — which is the ordinary case and
//! must stay the default, or the editor would wake on a timer for the rest of the session to ask a
//! question nobody is listening for.
//!
//! # Why it is not in `prompt`
//!
//! It was. The prompt was simply the first thing to need it, and the mechanism has nothing to do
//! with prompts: a ghost suggestion fetched over a network is the same shape, and so is a dropdown
//! gaining rows while it is open. Leaving it in `prompt` would have meant a suggestion provider
//! calling `prompt::refresh_started()` to say it was not a prompt.
//!
//! # Counted, not a flag
//!
//! Two runs can be outstanding at once — a prompt rebuilding while a provider is thinking — and a
//! flag cleared by whichever finished first would stop the editor waiting for the other.

use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

/// How many answers have been started and not yet finished.
static OUTSTANDING: AtomicUsize = AtomicUsize::new(0);

/// Bumped whenever something the frame draws has changed underneath it.
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// Say that an answer may arrive later, on some other thread.
///
/// Every caller must pair this with [`finished`] on **every** path out, including the failing ones:
/// a run that started and never finished leaves the editor polling for the rest of the session.
pub fn started() {
    OUTSTANDING.fetch_add(1, Ordering::SeqCst);
}

/// Say that one finished, however it ended.
pub fn finished() {
    let _ = OUTSTANDING.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |n| {
        Some(n.saturating_sub(1))
    });
}

/// Whether an answer may still arrive for what is already on screen.
pub fn outstanding() -> bool {
    OUTSTANDING.load(Ordering::SeqCst) > 0
}

/// Say that something the editor draws has changed.
pub fn landed() {
    GENERATION.fetch_add(1, Ordering::Relaxed);
}

/// The current generation. Equal to an earlier reading means nothing has landed since.
pub fn generation() -> u64 {
    GENERATION.load(Ordering::Relaxed)
}

/// Forget every outstanding answer. For a test, and for the moment a line is accepted — whatever
/// was still coming was for a line that no longer exists.
pub fn settle() {
    OUTSTANDING.store(0, Ordering::SeqCst);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The counters are process-wide, so the tests that move them run one at a time.
    static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn alone() -> std::sync::MutexGuard<'static, ()> {
        let guard = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
        settle();
        guard
    }

    #[test]
    fn nothing_outstanding_is_the_resting_state() {
        let _guard = alone();
        assert!(
            !outstanding(),
            "a shell that has asked nothing waits on a key"
        );
    }

    /// **Counted, not a flag.** Two runs at once, and the first to finish must not tell the editor
    /// to stop waiting for the second.
    #[test]
    fn two_answers_at_once_both_have_to_finish() {
        let _guard = alone();
        started();
        started();
        finished();
        assert!(outstanding(), "one is still coming");
        finished();
        assert!(!outstanding());
    }

    /// A stray `finished` must not wrap the counter round to a number that never empties.
    #[test]
    fn finishing_what_never_started_is_harmless() {
        let _guard = alone();
        finished();
        assert!(!outstanding());
        started();
        finished();
        assert!(!outstanding());
    }

    #[test]
    fn a_landing_is_visible_as_a_changed_generation() {
        let _guard = alone();
        let seen = generation();
        assert_eq!(generation(), seen, "nothing has landed yet");
        landed();
        assert_ne!(generation(), seen);
    }
}

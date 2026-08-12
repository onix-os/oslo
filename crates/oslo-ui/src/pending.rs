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
    WAKE_AT.with(|at| at.set(None));
}

thread_local! {
    /// The soonest moment something wants the editor to have another turn.
    ///
    /// **What makes a debounce possible at all.** A provider that waits for typing to stop has
    /// nothing to wait *with*: the editor blocks on a keystroke, so a delay measured from the last
    /// key would be noticed only when the next one arrived — which is exactly the key the delay
    /// exists to avoid asking about. This asks for a turn at a stated time.
    ///
    /// Thread-local because only the editor sets and reads it, unlike [`OUTSTANDING`], which a
    /// worker thread decrements when its answer lands.
    static WAKE_AT: std::cell::Cell<Option<std::time::Instant>> = const {
        std::cell::Cell::new(None)
    };
}

/// Ask for a turn in `after`, unless something already wants one sooner.
pub fn wake_in(after: std::time::Duration) {
    WAKE_AT.with(|at| {
        let want = std::time::Instant::now() + after;
        match at.get() {
            Some(already) if already <= want => {}
            _ => at.set(Some(want)),
        }
    });
}

/// Whether the editor should come up for air at all: an answer is coming, or a turn was asked for.
pub fn waiting() -> bool {
    outstanding() || WAKE_AT.with(|at| at.get().is_some())
}

/// Whether the moment asked for has arrived, clearing it if so.
///
/// Clearing here rather than leaving it to the caller: a turn that has been given is spent, and one
/// that stayed set would make the editor spin at the poll interval for the rest of the session.
pub fn due() -> bool {
    WAKE_AT.with(|at| match at.get() {
        Some(want) if std::time::Instant::now() >= want => {
            at.set(None);
            true
        }
        _ => false,
    })
}

/// How long the editor may block for, given `slice` as the longest it would otherwise wait.
///
/// Never longer than the turn somebody asked for, and never zero — a zero timeout is a spin.
pub fn slice_ms(slice: i32) -> i32 {
    let Some(want) = WAKE_AT.with(|at| at.get()) else {
        return slice;
    };
    let left = want
        .saturating_duration_since(std::time::Instant::now())
        .as_millis()
        .min(i32::MAX as u128) as i32;
    left.clamp(1, slice)
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

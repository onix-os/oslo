//! Hooks that fired at a moment when they could not have done anything.
//!
//! # The problem this exists for
//!
//! `post-change-dir` fires from `attempt_directory`, which runs with the shell's `Environment`
//! locked — every `cd`, `pushd` and jump reaches it through a `&mut Environment` that can only
//! have come from that lock. So a handler calling `oslo.env.set` or `oslo.source` met
//! [`super::borrow_env`]'s deliberate `try_lock` failure and did nothing, every time. Half the
//! twenty hooks fire from somewhere with that shape.
//!
//! Deferring is the only answer that keeps both halves: the fire *site* stays where it is accurate
//! — `attempt_directory` also catches a `cd` inside a function, which comparing directories across
//! a whole command line cannot — while the *handler* runs a moment later, with a shell it can
//! actually change.
//!
//! # Why not defer everything
//!
//! Two reasons, and they pull in opposite directions.
//!
//! A hook that **answers** cannot be deferred at all: `pre-cmd` returning `false` has to be read
//! while the command is still stoppable. Those keep running inline, and an answering handler that
//! reaches for shell state gets [`super::borrow_env`]'s error — which now says so.
//!
//! And a notifying hook fired from somewhere the state is *free* — `post-mode-change` from the
//! line editor, `pre-prompt` from the loop — must not be delayed either, or the prompt would show
//! the vi mode one command late. So the question is not which hook it is but whether the state is
//! held **right now**, which is what [`super::state_is_held`] answers.

use super::LuaEngine;
use oslo_base::value::Value;
use std::cell::{Cell, RefCell};

thread_local! {
    /// Hooks fired while the shell held its state, oldest first.
    ///
    /// Thread-local because the interpreter is: a hook deferred on this thread has to run against
    /// the interpreter parked here, and there is no other thread that could drain it.
    static PENDING: RefCell<Vec<(usize, Vec<Value>)>> = const { RefCell::new(Vec::new()) };

    /// Whether [`drain`] is already running further up the stack.
    ///
    /// A handler is free to do anything a config can, which includes running a command that
    /// changes directory — so draining can queue more work and re-enter here. Without this the
    /// second entry would drain what the first is part-way through, and a handler that `cd`s in a
    /// `post-change-dir` would recurse until the stack ran out.
    static DRAINING: Cell<bool> = const { Cell::new(false) };
}

/// Hold `args` until the shell can act on them.
pub(crate) fn defer(index: usize, args: Vec<Value>) {
    PENDING.with(|q| q.borrow_mut().push((index, args)));
}

/// Run everything held, if this is a moment when running it can work.
///
/// **A no-op unless the shell is idle**, which is what makes it safe to call from anywhere: the
/// two conditions that would make a deferred hook fail again — the state still held, or a drain
/// already in progress — leave the queue exactly as it was for the next opportunity. The empty
/// case is one thread-local read, so the call sites do not need to guess whether anything is
/// waiting.
pub fn drain() {
    if PENDING.with(|q| q.borrow().is_empty()) || DRAINING.get() || super::state_is_held() {
        return;
    }
    DRAINING.set(true);
    // Taken whole rather than popped one at a time: a handler that queues more work leaves it for
    // the next drain instead of extending the walk it is inside.
    let ready = PENDING.with(|q| std::mem::take(&mut *q.borrow_mut()));
    for (index, args) in ready {
        super::hooks::fire_now(index, args);
    }
    DRAINING.set(false);
}

/// A hook's fields as the value a handler is called with.
pub(crate) fn fields(pairs: &[(&str, &str)]) -> Vec<Value> {
    vec![LuaEngine::hook_fields(
        &pairs
            .iter()
            .map(|(name, value)| (*name, Value::str(value)))
            .collect::<Vec<_>>(),
    )]
}

//! Taking the shell's state for one `oslo.*` call, and giving it back.

use super::queue;

use oslo_base::value::{LuaError, LuaResult};
use oslo_shell::env::Environment;
use std::sync::{Mutex, MutexGuard};

/// Take the shell state for the duration of one `oslo.*` call.
///
/// `try_lock`, not `lock`: the interpreter runs *inside* the evaluator when a Lua-registered
/// builtin executes, and the evaluator is already holding this mutex. `lock` there is a hang —
/// the shell stops responding with no output at all. A Lua error instead is recoverable and says
/// what happened.
///
/// **Anything deferred runs first.** Every `oslo.*` function that touches the shell comes through
/// here, so this is where a chunk picks up hooks queued by an earlier call. The release does the
/// same on the way out — see [`EnvBorrow`], which is the one that matters.
pub(crate) fn borrow_env(env: &Mutex<Environment>) -> LuaResult<EnvBorrow<'_>> {
    queue::drain();
    env.try_lock().map(EnvBorrow::new).map_err(|_| {
        // **Naming both causes, because the second one cost a day.** This said only
        // `oslo.register_builtin`, so a `post-change-dir` handler that could not set a variable
        // reported a builtin registration nobody had written — which reads as noise and gets
        // skipped over. Hooks that answer run inline by necessity and are the common way to arrive
        // here now.
        LuaError::new(
            "oslo: shell state is busy; an oslo.* call that reaches the shell cannot run from \
             here. This is a builtin registered with oslo.register_builtin, or a hook that \
             answers — pre-cmd, pre-change-dir, on-command-not-found — which must run while the \
             shell holds its state. Hooks that only observe are handed everything they need and \
             may use the whole API.",
        )
    })
}

/// The shell state, held for one `oslo.*` call, that drains deferred hooks when it is let go.
///
/// # Why the release is the drain point
///
/// A hook deferred because the state was locked has to run when it is not, and the moment that
/// becomes true is exactly the moment this is dropped. Putting the drain here rather than at each
/// of the thirty call sites is the difference between a rule and a habit: `oslo.run{"cd", d}`
/// leaves the handler run by the time it returns, and so does every call added later, without
/// anyone remembering to say so.
///
/// The guard is released **before** the drain, not after — a handler is free to use the whole
/// `oslo.*` API, which means taking this same lock. Dropping in the other order is the deadlock
/// this type exists to avoid.
pub(crate) struct EnvBorrow<'a>(Option<MutexGuard<'a, Environment>>);

impl<'a> EnvBorrow<'a> {
    fn new(guard: MutexGuard<'a, Environment>) -> Self {
        Self(Some(guard))
    }
}

impl std::ops::Deref for EnvBorrow<'_> {
    type Target = Environment;
    fn deref(&self) -> &Environment {
        self.0.as_ref().expect("the guard is taken only on drop")
    }
}

impl std::ops::DerefMut for EnvBorrow<'_> {
    fn deref_mut(&mut self) -> &mut Environment {
        self.0.as_mut().expect("the guard is taken only on drop")
    }
}

impl Drop for EnvBorrow<'_> {
    fn drop(&mut self) {
        drop(self.0.take());
        queue::drain();
    }
}

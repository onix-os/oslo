//! The reach-back from anywhere in the shell into whatever interpreter is on this thread.
//!
//! A hook fires from places a long way from where the engine is owned — `run_simple` has no route
//! back to `repl`, and neither does the `cd` builtin or the line editor. The interpreter is parked
//! on this thread in [`super::ACTIVE`] for exactly that, so these are free functions rather than
//! methods: the caller has no `LuaEngine` to call one on.
//!
//! Three shapes, and which one a moment uses is fixed by `lua::api::hooks`:
//!
//! * [`fire_at_here`] tells every handler and reads nothing back.
//! * [`answer_hook_with`] asks, and the first handler to return something has decided.
//! * [`ask_hook_here`] asks for a *status*, which is what `on-command-not-found` answers with.
//!
//! Every one of them checks the watched bit first, so a moment nobody attached to costs a single
//! relaxed load. That is what lets `fire_at_here` sit on the keystroke and mode-change paths.

use super::ACTIVE;
use crate::lua::engine::Registry;
use oslo_lua::Interp;
use oslo_lua::value::Value;

/// Fire an answering hook using whatever interpreter is on this thread.
///
/// The executor is a long way from the place the engine is owned — `run_simple` has no route back
/// to `repl` — but the interpreter is already parked on this thread for exactly this kind of
/// reach-back. `None` when there is no interpreter (a non-interactive shell, a script) or when no
/// handler answered, and the caller then does what it did before.
///
/// **By index, not by name, and that is a fix rather than tidying.** The one caller asked for
/// `"command-not-found"`, which is an *alias*; handlers are stored under the canonical name, so the
/// lookup found an empty list and the hook never fired under either spelling. Every other fire site
/// already named the moment by index for this reason.
pub fn ask_hook_here(index: usize, args: Vec<Value>) -> Option<i32> {
    debug_assert!(
        crate::lua::api::hooks::HOOKS[index].answers,
        "ask_hook_here is for a hook whose answer is read; HOOKS says this one only observes"
    );
    if !crate::lua::api::hooks::watched(index) {
        return None;
    }
    let name = crate::lua::api::hooks::HOOKS[index].name;
    let (interp, registry) = ACTIVE.with(|slot| slot.borrow().clone())?;
    ask_hook_on(&interp, &registry, name, args)
}

/// Whether the `key` hook has anything attached, and is therefore worth building a payload for.
///
/// Re-exported here because the editor lives in the binary and cannot see `lua::api`. See
/// `api::shell::KEY_WATCHED` for why the question is asked this way rather than by looking.
pub fn key_hook_watched() -> bool {
    crate::lua::api::hooks::watched(crate::lua::api::hooks::at::ON_KEY)
}

/// Fire the `key` hook, answering with the first handler that returned anything.
///
/// **First to answer wins, and the rest are skipped** — the same rule `ask_hook_here` uses, and
/// the right one for an interception chain: the handler that decided what the key means has ended
/// the question. A handler that only watched returns nothing and the next one is asked.
///
/// `None` when nothing is attached, when there is no interpreter on this thread, or when every
/// handler declined. The caller then does what it would have done without the hook, which is what
/// keeps a config that only *observes* keys from changing how any of them behave.
pub fn key_hook_here(args: Vec<Value>) -> Option<Value> {
    answer_hook_with(crate::lua::api::hooks::at::ON_KEY, args)
}

/// Fire a hook that may *answer*, by index, and hand back the first answer given.
///
/// The shape `on-key`, `pre-cmd` and `pre-change-dir` share: a handler that returns nothing has
/// only watched, and the first one to return something has decided. Gated on the watched bit so a
/// hook nobody attached to costs one relaxed load — which is what makes it usable on the keystroke
/// path.
pub fn answer_hook_here(index: usize) -> Option<Value> {
    answer_hook_with(index, Vec::new())
}

/// Tell a hook something happened, from wherever the shell happens to be.
///
/// The notifying twin of [`answer_hook_with`], and the same reach-back: the executor and the `cd`
/// builtin are a long way from the place the engine is owned, but the interpreter is parked on
/// this thread for exactly this. Fields are `(name, value)` pairs because every caller has strings.
pub fn fire_at_here(index: usize, fields: &[(&str, &str)]) {
    if !crate::lua::api::hooks::watched(index) {
        return;
    }
    fire_or_defer(index, super::queue::fields(fields));
}

/// [`fire_at_here`] for a caller that already has the handler's arguments as values.
pub(crate) fn fire_values_here(index: usize, args: Vec<Value>) {
    if !crate::lua::api::hooks::watched(index) {
        return;
    }
    fire_or_defer(index, args);
}

/// Run a notifying hook now, or hold it until the shell can act on it.
///
/// **The whole of the inline-versus-deferred decision, in one place.** A hook that only observes
/// has no reason to run while the shell is mid-command with its state locked — there it can look
/// but not touch, which is the defect this exists to close — and every reason to run the moment
/// that is no longer true. Where the state is already free, which is most fire sites, nothing
/// changes: it runs inline exactly as before.
fn fire_or_defer(index: usize, args: Vec<Value>) {
    debug_assert!(
        !crate::lua::api::hooks::HOOKS[index].answers,
        "this hook's answer is read; fire_at_here would discard it — use answer_hook_with"
    );
    if super::state_is_held() {
        super::queue::defer(index, args);
        return;
    }
    fire_now(index, args);
}

/// Call every handler attached to a notifying hook, here and now.
///
/// The bottom of both paths: [`fire_or_defer`] when the shell is idle, and the queue when it has
/// become idle since. Nothing checks the watched bit or the lock again — both callers have.
pub(super) fn fire_now(index: usize, args: Vec<Value>) {
    let name = crate::lua::api::hooks::HOOKS[index].name;
    let Some((interp, registry)) = ACTIVE.with(|slot| slot.borrow().clone()) else {
        return;
    };
    for handler in crate::lua::api::hook_handlers(&registry, name) {
        if let Err(e) = interp.call(&handler, args.clone()) {
            oslo_base::messages::error(format!("{name} hook"), e.to_string());
        }
    }
}

/// [`answer_hook_here`], with arguments for the handler.
pub fn answer_hook_with(index: usize, args: Vec<Value>) -> Option<Value> {
    // The other half of the split `fire_or_defer` asserts. Between them, `Hook.answers` is what
    // decides how a moment is dispatched rather than a second list that can drift from it — which
    // is the state that let `on-command-not-found` be asked for by a name nothing stored under.
    debug_assert!(
        crate::lua::api::hooks::HOOKS[index].answers,
        "this hook only observes; its return value would be read by nobody"
    );
    if !crate::lua::api::hooks::watched(index) {
        return None;
    }
    let name = crate::lua::api::hooks::HOOKS[index].name;
    let (interp, registry) = ACTIVE.with(|slot| slot.borrow().clone())?;
    for handler in crate::lua::api::hook_handlers(&registry, name) {
        match interp.call(&handler, args.clone()) {
            Ok(values) => match values.first() {
                None | Some(Value::Nil) => {}
                Some(answer) => return Some(answer.clone()),
            },
            // Reported and skipped. A handler that raises on every keystroke would otherwise fill
            // the screen faster than it could be read, but silence here means a hook that has
            // stopped working with nothing to say why — and this one is on the typing path, so
            // "it went quiet" is exactly the symptom that needs explaining.
            Err(e) => oslo_base::messages::error("key hook", e.to_string()),
        }
    }
    None
}

pub(super) fn ask_hook_on(
    interp: &Interp,
    registry: &Registry,
    name: &str,
    args: Vec<Value>,
) -> Option<i32> {
    for handler in crate::lua::api::hook_handlers(registry, name) {
        match interp.call(&handler, args.clone()) {
            Ok(values) => {
                if let Some(Value::Number(n)) = values.first() {
                    return n.as_int().map(|i| i as i32);
                }
            }
            // Reported and skipped, as with any other hook: one broken handler must not stop the
            // others, and must not turn a missing command into a silent success.
            Err(e) => oslo_base::messages::error(format!("{name} hook"), e.to_string()),
        }
    }
    None
}

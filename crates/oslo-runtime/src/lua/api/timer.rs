//! `oslo.after` and `oslo.every` — the only things in oslo that mean "later".
//!
//! ```lua
//! oslo.after(500, function() … end)          -- once, in half a second
//! local t = oslo.every(30000, function() … end)
//! t:stop()
//! ```
//!
//! # Fired by the read loop, not by an event loop
//!
//! **A timer fires between commands, never during one, and never while you are typing.** The loop
//! checks what is due at the top of each iteration and after each command finishes; those are the
//! two moments the shell holds nothing and can safely call Lua, and they are already where deferred
//! hooks drain.
//!
//! **And at an idle prompt.** A prompt sitting untouched is a third such moment: the shell holds
//! nothing there either, and it is where a timer set for "in five minutes" would otherwise have
//! waited for the next keystroke to be noticed — the one moment its author did not mean.
//! [`next_due_in_ms`] is what the editor's wait is given instead of "for ever", so the wake happens
//! when the timer says rather than when the terminal does.
//!
//! The alternative is a real event loop — neovim has one, and `vim.uv` timers fire whenever it turns.
//! That means libuv or `tokio` in a shell that deliberately removed `tokio`, plus every Lua callback
//! becoming a thing that can run in the middle of an expansion. The trade taken here is the honest
//! one: a smaller promise, kept exactly.
//!
//! So `oslo.every(1000, …)` at an idle prompt does not tick once a second. It ticks the next time
//! you run something. A timer is for "not now" and "not on every prompt", not for a clock.

use super::util::{ok, put};
use oslo_base::value::{LuaError, LuaResult};
use oslo_base::value::{Table, Value};
use std::cell::RefCell;
use std::time::{Duration, Instant};

/// One timer waiting to fire.
struct Timer {
    id: u64,
    /// When it is next due.
    due: Instant,
    /// How often it repeats, or `None` for a one-shot.
    every: Option<Duration>,
    handler: Value,
}

thread_local! {
    static TIMERS: RefCell<Vec<Timer>> = const { RefCell::new(Vec::new()) };
    static NEXT_ID: RefCell<u64> = const { RefCell::new(1) };
}

/// The longest a timer may be set for: a day.
///
/// Not a limit anybody should meet. It exists because `oslo.after(1e30, …)` would otherwise
/// overflow the instant arithmetic, and a shell that panics because a config asked for a long wait
/// is worse than one that says the wait is too long.
const LONGEST: Duration = Duration::from_secs(24 * 60 * 60);

/// Add `oslo.after` and `oslo.every`.
pub fn install(oslo: &mut Table) {
    put(oslo, "after", |_, args| {
        schedule(&args, "oslo.after", false)
    });
    put(oslo, "every", |_, args| schedule(&args, "oslo.every", true));
}

/// Read `(milliseconds, function)` and put a timer in the list.
fn schedule(args: &[Value], called: &str, repeating: bool) -> LuaResult<Vec<Value>> {
    let Some(millis) = args.first().and_then(Value::as_number) else {
        return Err(LuaError::new(format!(
            "{called}: the first argument is a delay in milliseconds"
        )));
    };
    let Some(handler @ Value::Function(_)) = args.get(1) else {
        return Err(LuaError::new(format!(
            "{called}: the second argument must be a function"
        )));
    };
    let millis = millis.as_float();
    if !(millis.is_finite() && millis >= 0.0) {
        return Err(LuaError::new(format!("{called}: {millis} is not a delay")));
    }
    let wait = Duration::from_secs_f64(millis / 1000.0).min(LONGEST);
    // **A repeating timer of zero would be due forever.** Every check would fire it again and the
    // loop would make no progress, so the smallest repeat is one millisecond.
    let wait = if repeating {
        wait.max(Duration::from_millis(1))
    } else {
        wait
    };

    let id = NEXT_ID.with(|next| {
        let mut next = next.borrow_mut();
        let id = *next;
        *next += 1;
        id
    });
    TIMERS.with(|timers| {
        timers.borrow_mut().push(Timer {
            id,
            due: Instant::now() + wait,
            every: repeating.then_some(wait),
            handler: handler.clone(),
        })
    });
    ok(handle(id))
}

/// What a timer answers with: `t:stop()` and nothing else.
fn handle(id: u64) -> Value {
    let mut table = super::handle::Handle::new("oslo.timer");

    table.verb("stop", move |_, _| ok(Value::Bool(forget(id))));

    // `<close>` stops it, the collector does not: `oslo.every` is written for its effect and its
    // handle is usually dropped, so a `__gc` that stopped would stop nearly every timer in the
    // shell. See [`super::handle::Handle::on_close`].
    table.on_close("oslo.timer.close", move || {
        forget(id);
    });

    table.build()
}

/// Take timer `id` out of the list, answering whether it was there.
fn forget(id: u64) -> bool {
    TIMERS.with(|timers| {
        let mut timers = timers.borrow_mut();
        let before = timers.len();
        timers.retain(|timer| timer.id != id);
        before != timers.len()
    })
}

/// Run whatever is due. Called by the read loop where nothing is held.
///
/// **The list is taken before anything runs.** A handler may set another timer — a poll that
/// reschedules itself is the ordinary case — and appending to a list being iterated is how that
/// becomes either a panic or an accidental infinite loop.
/// Answers whether anything actually ran, which the caller needs to decide whether the prompt is
/// still true — a handler that changed what `oslo.prompt.left` reads leaves a drawn prompt stale.
pub fn fire_due() -> bool {
    let due = settle(Instant::now());
    let ran = !due.is_empty();
    for (id, handler) in due {
        if let Err(problem) = crate::lua::engine::call_here(&handler, Vec::new()) {
            eprintln!("oslo: timer: {problem}");
            // A repeating timer that raises is stopped rather than left to raise on every command
            // for the rest of the session.
            TIMERS.with(|timers| timers.borrow_mut().retain(|timer| timer.id != id));
        }
    }
    ran
}

/// Take what is due at `now`, leaving the list as it should be afterwards.
///
/// **The list is settled before any handler runs**, and that is the whole of this function. A
/// handler may set another timer — a poll that reschedules itself is the ordinary case — so
/// iterating the live list would be either a panic on the outstanding borrow or an accidental
/// infinite loop. Dropping a one-shot here rather than after also means a handler that raises cannot
/// leave it in the list to fire again on the next command, forever.
///
/// Separate from [`fire_due`] because it is the half that can be tested: firing needs an interpreter
/// on the thread, and the bookkeeping needs nothing.
fn settle(now: Instant) -> Vec<(u64, Value)> {
    let due: Vec<(u64, Value)> = TIMERS.with(|timers| {
        timers
            .borrow()
            .iter()
            .filter(|timer| timer.due <= now)
            .map(|timer| (timer.id, timer.handler.clone()))
            .collect()
    });
    if due.is_empty() {
        return due;
    }
    TIMERS.with(|timers| {
        timers.borrow_mut().retain_mut(|timer| match timer.every {
            Some(every) if timer.due <= now => {
                // From now rather than from when it was due: a shell can sit at a prompt for an
                // hour, and catching up on sixty missed ticks at once is never what was wanted.
                timer.due = now + every;
                true
            }
            Some(_) => true,
            None => timer.due > now,
        })
    });
    due
}

/// Whether anything is waiting, so the loop can skip the check entirely.
pub fn any() -> bool {
    TIMERS.with(|timers| !timers.borrow().is_empty())
}

/// How long until the soonest timer is due, in milliseconds. `None` when nothing is waiting.
///
/// **For the editor's wait, so a timer fires while you sit there.** A timer used to come due only
/// at a command boundary: set one for five seconds, touch nothing, and it fired when you next
/// pressed Enter — which is the one moment its author did not mean. This is the deadline the idle
/// wait is given instead of "for ever".
///
/// Zero for something already due, which is a poll that returns at once rather than a negative
/// timeout, which `poll` reads as "no timeout at all".
pub fn next_due_in_ms() -> Option<u64> {
    let now = Instant::now();
    TIMERS.with(|timers| {
        timers
            .borrow()
            .iter()
            .map(|timer| timer.due.saturating_duration_since(now).as_millis() as u64)
            .min()
    })
}

#[cfg(test)]
#[path = "timer/tests.rs"]
mod tests;

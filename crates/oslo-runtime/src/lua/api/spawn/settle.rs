//! Waiting for a spawn, for the places that have no idle loop to wait in.
//!
//! ```lua
//! local job = oslo.spawn{ "cargo", "build" }
//! local out, status = job:wait()          -- block until this one is done
//! oslo.settle{ timeout_ms = 60000 }       -- block until every spawn is
//! ```
//!
//! # The bug this closes
//!
//! `oslo.spawn` was **unusable inside `oslo make`**, silently. A worker queues its result and calls
//! `background::nudge`; the byte goes into a self-pipe that only the line editor's `poll` watches.
//! A `.make.lua` never enters a REPL, so nothing ever looked — the callback simply never ran, and
//! the recipe ended having quietly done nothing in parallel. The other half of the fix is
//! `src/cli/make.rs` arming the pipe and installing a servicer at all.
//!
//! # Why this is not a sleep loop
//!
//! The proposal that led here suggested polling every 2 ms. That reinvents what
//! [`oslo_base::background`] already is: the wakers are real descriptors, `nudge` writes to one,
//! and `poll` returns the instant a worker finishes rather than up to 2 ms later. The same wait the
//! editor does, minus the terminal.

use super::super::util::{ok, put, record};
use oslo_base::value::{Table, Value};
use std::time::{Duration, Instant};

/// Add `oslo.settle`.
pub fn install(oslo: &mut Table) {
    // oslo.settle{ timeout_ms = n } -> { fired = n, outstanding = n, settled = bool }
    //
    // `settled` is the question a recipe actually asks — "is everything done?" — and answering it
    // with `outstanding == 0` at every call site is how a timeout gets ignored by accident.
    put(oslo, "settle", |_, args| {
        let deadline = deadline_from(args.first(), "oslo.settle")?;
        let mut fired = 0;
        loop {
            fired += super::deliver_counting();
            if super::outstanding() == 0 {
                break;
            }
            if !block_once(deadline) {
                break;
            }
        }
        let left = super::outstanding();
        ok(record(vec![
            ("fired", Value::int(fired as i64)),
            ("outstanding", Value::int(left as i64)),
            ("settled", Value::Bool(left == 0)),
        ]))
    });
}

/// `job:wait([timeout_ms])` -> out, status — or `nil, why`.
///
/// **The deadline is argument two.** A `:` call hands the handle in first, the same as every other
/// verb — reading `args[0]` here got the handle, and a table is not a number, so every `wait` ran
/// with no deadline at all and blocked until the process ended.
pub(super) fn wait(id: u64, args: &[Value]) -> oslo_base::value::LuaResult<Vec<Value>> {
    let deadline = deadline_from(args.get(1), "oslo.spawn:wait")?;
    loop {
        if let Some((out, status)) = super::claim(id) {
            return Ok(vec![Value::str(out), Value::int(status as i64)]);
        }
        // **Not live and not in the queue means somebody already took it** — an `on_exit` that ran
        // at a command boundary, or a `cancel`. Answering `nil` with a reason beats blocking until
        // the timeout for a result that is never coming.
        if !super::is_live(id) {
            return Ok(vec![
                Value::Nil,
                Value::str("already delivered, cancelled, or never started"),
            ]);
        }
        if !block_once(deadline) {
            return Ok(vec![Value::Nil, Value::str("timed out")]);
        }
    }
}

/// `{ timeout_ms = n }` or a bare number, as an instant to give up at.
fn deadline_from(
    value: Option<&Value>,
    owner: &str,
) -> oslo_base::value::LuaResult<Option<Instant>> {
    let ms = match value {
        None | Some(Value::Nil) => None,
        Some(Value::Table(opts)) => opts.borrow().get_str("timeout_ms").as_number(),
        Some(Value::Number(n)) => Some(*n),
        Some(other) => {
            return Err(oslo_base::value::LuaError::new(format!(
                "{owner}: expects a number of milliseconds or a table, got {}",
                other.type_name()
            )));
        }
    };
    Ok(ms
        .map(|n| n.as_float())
        .filter(|ms| *ms > 0.0)
        .map(|ms| Instant::now() + Duration::from_secs_f64(ms / 1000.0)))
}

/// Block until something happens or the deadline passes. `false` means the deadline passed.
///
/// The wakers are the same set the editor polls, so a finished worker's `nudge` returns this at
/// once. **With no wakers registered there is nothing to poll**, which happens when nobody armed
/// the pipe; a short sleep then keeps the loop from spinning a core while it waits for a result
/// that is still arriving through the queue.
fn block_once(deadline: Option<Instant>) -> bool {
    let left_ms = match deadline {
        Some(at) => {
            let left = at.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            i32::try_from(left.as_millis()).unwrap_or(i32::MAX)
        }
        None => -1,
    };
    let wakers = oslo_base::background::wakers();
    if wakers.is_empty() {
        std::thread::sleep(Duration::from_millis(if left_ms < 0 {
            5
        } else {
            left_ms.min(5) as u64
        }));
        return true;
    }
    let mut fds: Vec<nix::libc::pollfd> = wakers
        .iter()
        .map(|fd| nix::libc::pollfd {
            fd: *fd,
            events: nix::libc::POLLIN,
            revents: 0,
        })
        .collect();
    // SAFETY: every descriptor is borrowed and open for the life of the shell, and the slice is
    // live for the call. The same contract `term::input::idle` polls under.
    let polled = unsafe { nix::libc::poll(fds.as_mut_ptr(), fds.len() as _, left_ms) };
    // A descriptor that fired is drained here, or the next poll returns on the same byte for ever.
    if polled > 0 {
        for fd in fds.iter().filter(|fd| fd.revents & nix::libc::POLLIN != 0) {
            oslo_base::background::read_waker(fd.fd);
        }
    }
    // `polled == 0` is the timeout; anything else — including `EINTR` — is worth another look.
    polled != 0 || deadline.is_none()
}

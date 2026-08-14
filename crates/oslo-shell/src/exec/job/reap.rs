//! Collecting the dead: the opportunistic reaper, and the sweep only PID 1 does.
//!
//! Split from [`super::table`] because they answer different questions. The table is what the shell
//! *remembers* about its jobs; this is what it owes the kernel — every child of this process stays
//! a zombie until this process waits for it, whether or not any job still names it.

use super::table::{LIVE_CHILDREN, Transition, with_jobs};
use nix::sys::wait::{WaitPidFlag, WaitStatus, waitpid};
use nix::unistd::Pid;
use std::sync::atomic::{AtomicU8, Ordering};

/// Collect every child that has ended or stopped since the last command, and report what changed.
///
/// R7.4: this is what keeps `sleep 0 &` from leaving a `[oslo] <defunct>` entry behind for the
/// rest of the session. It never blocks — `WNOHANG` — so a still-running job costs one syscall.
///
/// Each known pid is asked about *by name* rather than with a single `waitpid(-1)`. Both collect
/// the same zombies, but `-1` also collects children this table has never heard of, and there is
/// always at least one class of those: a `fork` whose parent is about to wait for it itself. In
/// the shell those two never overlap — a command boundary is not a moment when a foreground child
/// is outstanding — but the library is also used from test binaries that evaluate scripts on
/// several threads at once, where `-1` reaped another thread's child and left its `waitpid` with
/// `ECHILD`. Naming the pids makes the safety local instead of global.
/// Reap children this shell never started, which only PID 1 has.
///
/// A double-forked process is reparented to init when its parent exits, and init is the only thing
/// that can reap it. An `/bin/sh` serving as an initramfs init that skips this fills the process
/// table with zombies — measured in the Alpine VM, which reported one left behind after every
/// double fork.
///
/// Gated on being process 1 rather than done unconditionally, and that is not caution about cost:
/// `waitpid(-1)` reaps *any* child, so an ordinary shell doing it could consume the status that a
/// `wait $pid` was about to read. Init has no such caller, and no other process inherits orphans.
///
/// A status that turns out to belong to a known job is still recorded, so `$!` and `jobs` stay
/// right if the sweep gets there first.
fn reap_orphans_as_init() {
    if !is_init() {
        return;
    }
    with_jobs(|jobs| {
        loop {
            match waitpid(Pid::from_raw(-1), Some(WaitPidFlag::WNOHANG)) {
                // Nothing has exited yet; the remaining children are still running.
                Ok(WaitStatus::StillAlive) => break,
                Ok(status) => jobs.record(status),
                Err(nix::errno::Errno::EINTR) => continue,
                // ECHILD: no children at all, which is the normal state for an idle init.
                Err(_) => break,
            }
        }
    });
}

/// Whether this process is PID 1, answered once rather than asked of the kernel per command.
///
/// **This was a `getpid(2)` on every command boundary** — measured at one syscall per simple
/// command, including bare assignments — spent to discover that a shell is not init, which is the
/// answer for every shell but one and never changes within a process.
///
/// `forgot_which_process_i_am` is what keeps that safe across `fork`: a child inherits this cache
/// along with everything else, and a child of init is *not* init. Every forked child already calls
/// [`super::reset_signals_for_child`], so the reset has one place to live.
fn is_init() -> bool {
    match KNOWN_INIT.load(Ordering::Relaxed) {
        0 => {
            let init = nix::unistd::getpid().as_raw() == 1;
            KNOWN_INIT.store(if init { 2 } else { 1 }, Ordering::Relaxed);
            init
        }
        2 => true,
        _ => false,
    }
}

/// Forget the cached answer, because this process is no longer the one that computed it.
pub(crate) fn forgot_which_process_i_am() {
    KNOWN_INIT.store(0, Ordering::Relaxed);
}

/// 0 = not yet asked, 1 = not init, 2 = init.
static KNOWN_INIT: AtomicU8 = AtomicU8::new(0);

pub fn reap_background_jobs() {
    // Orphans first, and *before* the early return below: as PID 1 there are children to reap even
    // when this shell started none of its own, which is exactly the case that early return skips.
    reap_orphans_as_init();
    if LIVE_CHILDREN.load(Ordering::SeqCst) == 0 {
        return;
    }
    let flags = WaitPidFlag::WNOHANG | WaitPidFlag::WUNTRACED | WaitPidFlag::WCONTINUED;
    let pending = with_jobs(|jobs| {
        for pid in jobs.reapable_pids() {
            loop {
                match waitpid(pid, Some(flags)) {
                    // `StillAlive` is what `WNOHANG` reports for a child with nothing to say yet.
                    Ok(WaitStatus::StillAlive) => break,
                    Ok(status) => {
                        jobs.record(status);
                        break;
                    }
                    // EINTR: a signal arrived instead — R7.6 removed `SA_RESTART`, so this is now
                    // a normal outcome rather than an impossible one.
                    Err(nix::errno::Errno::EINTR) => continue,
                    // ECHILD: already reaped, or never ours. Either way there is nothing left to
                    // wait for, and the entry must stop being counted or the fast path never
                    // rearms. No status is invented for it — `wait` reporting "unknown pid" is
                    // honest, where reporting 0 would be a fabricated success.
                    Err(_) => {
                        jobs.abandon(pid);
                        break;
                    }
                }
            }
        }
        super::report::announce_changes(jobs);
        jobs.take_transitions()
    });
    // **Outside `with_jobs`, and that is the whole point.** A handler is entitled to ask the shell
    // about its jobs — `oslo.job.list()` takes this same lock — so firing from inside would be a
    // handler waiting for a lock its own caller holds.
    announce(pending);
}

/// Tell the hooks what the reaper found, now that the table is not locked.
fn announce(pending: Vec<Transition>) {
    use oslo_base::hooks::{at, fire_at_here, watched};
    if pending.is_empty() || !(watched(at::PROCESS_EXIT) || watched(at::JOB_STATE)) {
        return;
    }
    for change in pending {
        match change {
            Transition::ProcessExit { pid, job, status } => fire_at_here(
                at::PROCESS_EXIT,
                &[
                    ("pid", &pid.as_raw().to_string()),
                    ("job", &job.map(|id| id.to_string()).unwrap_or_default()),
                    ("status", &status.to_string()),
                ],
            ),
            Transition::JobState {
                id,
                pgid,
                text,
                from,
                to,
                background,
            } => fire_at_here(
                at::JOB_STATE,
                &[
                    ("id", &id.to_string()),
                    ("pid", &pgid.as_raw().to_string()),
                    ("text", &text),
                    ("from", from),
                    ("to", to),
                    ("background", if background { "1" } else { "" }),
                ],
            ),
        }
    }
}

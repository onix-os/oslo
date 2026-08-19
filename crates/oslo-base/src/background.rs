//! Servicing background work from wherever the shell happens to be waiting.
//!
//! # The problem this solves
//!
//! An idle editor is blocked waiting for the terminal. A child exiting, or another shell writing a
//! universal variable, is not a keystroke — so without help the shell learns about it at the *next
//! command boundary*, which for somebody sitting at a prompt means "when you next press Enter". A
//! job that finished ten minutes ago is announced when you run the next thing.
//!
//! # Two ways in, because there are two kinds of event
//!
//! * **A signal**, for a child ending. The handler writes one byte down the self-pipe below rather
//!   than letting `EINTR` do the waking, which is the route `SIGWINCH` takes — see
//!   [`crate::background::nudge_from_signal`] and the measurement in `oslo_ui::term::child`.
//! * **A descriptor**, for anything a signal cannot carry: an `inotify` watch on the universal
//!   store, a worker finishing a spawn. Registered here and waited on beside the terminal.
//!
//! So both ends up being the same thing — a descriptor the idle wait already polls — and the wait
//! has exactly one way to be woken instead of two that have to agree.
//!
//! Neither carries data. Both mean *something changed, go and look* — [`crate::background::service`] is what looks,
//! and it runs on the shell thread with the locks free.
//!
//! # Why the servicer is installed rather than called
//!
//! The editor is in `oslo-ui`, which sits *below* `oslo-shell` in the crate graph: it cannot name
//! the job table or the environment, and should not. The same inversion as [`crate::hooks`] — the
//! layer that can do the work installs it, the layer that knows *when* calls it.
//!
//! A closure rather than a function pointer, because the work needs state the installer holds: the
//! environment a universal refresh writes into is owned by the REPL.
//!
//! Before installation everything here is a no-op, which is what `sh -c` and every script want.

use std::sync::{Mutex, OnceLock, RwLock};

/// What to run when something in the background may have changed.
static SERVICE: OnceLock<Box<dyn Fn() -> bool + Send + Sync>> = OnceLock::new();

/// Descriptors to wait on beside the terminal, each with the reader that decides whether the event
/// was one anybody asked for. Read-mostly: written once each at startup, read on every editor wait.
static WAKERS: RwLock<Vec<Waker>> = RwLock::new(Vec::new());

/// One thing worth waking for.
struct Waker {
    fd: i32,
    /// Empties the descriptor and answers whether what it held was interesting.
    ///
    /// **Because a watch is coarser than the thing being watched.** `inotify` watches a directory,
    /// and the shell writes its history, its model and its macros into the same one — so most wakes
    /// are the shell hearing itself. Servicing every one of them is cheap but not free, and it made
    /// the incidental traffic look like the mechanism under test.
    read: fn(i32) -> bool,
}

/// Install the servicer. The first call wins; a later one is ignored rather than panicking, because
/// a second install is a startup-order mistake and not worth killing a shell over.
pub fn install(service: impl Fn() -> bool + Send + Sync + 'static) {
    let _ = SERVICE.set(Box::new(service));
}

/// Do whatever the background needs, if anybody is able to.
///
/// **On the shell thread, between keystrokes, with no editor borrow held** — the same footing as a
/// hook, so it may take locks, reap children, print a notice and run Lua. Never call it from a
/// signal handler: a handler's whole job is to set a flag and let the blocked `read` fail.
///
/// Answers whether anything visible changed, so the caller knows whether a repaint is owed. Most
/// wakes are the shell hearing its own writes to a watched directory, and repainting on every one
/// of those would redraw the prompt after every command.
pub fn service() -> bool {
    match SERVICE.get() {
        Some(service) => service(),
        None => false,
    }
}

/// Whether anything is installed — for a caller deciding whether a wake is worth arranging at all.
pub fn is_installed() -> bool {
    SERVICE.get().is_some()
}

/// Wait on `fd` alongside the terminal, and service the background when it becomes readable.
///
/// The descriptor is **borrowed, not owned**: whoever registers it keeps it open for the life of the
/// shell. A watch that closes would leave a descriptor number here that the kernel has since given
/// to something else, and the editor would wake on somebody else's traffic for ever.
pub fn wake_on(fd: i32) {
    wake_on_with(fd, |fd| {
        drain(fd);
        true
    });
}

/// The same, for a descriptor whose events have to be looked at before they mean anything.
pub fn wake_on_with(fd: i32, read: fn(i32) -> bool) {
    if let Ok(mut wakers) = WAKERS.write()
        && !wakers.iter().any(|waker| waker.fd == fd)
    {
        wakers.push(Waker { fd, read });
    }
}

/// Every registered descriptor, for the wait.
pub fn wakers() -> Vec<i32> {
    WAKERS
        .read()
        .map(|wakers| wakers.iter().map(|waker| waker.fd).collect())
        .unwrap_or_default()
}

/// Empty `fd` and answer whether what it held is worth servicing for.
pub fn read_waker(fd: i32) -> bool {
    let read = WAKERS
        .read()
        .ok()
        .and_then(|wakers| wakers.iter().find(|waker| waker.fd == fd).map(|w| w.read));
    match read {
        Some(read) => read(fd),
        None => false,
    }
}

/// Drain a waker that has fired, so the next wait does not fire again on the same event.
///
/// **Whatever it is.** An `inotify` descriptor hands back a struct per event and a pipe hands back
/// bytes; neither is read for its content, because the content is always the same message — go and
/// look. Reading until it would block is the only thing both need.
pub fn drain(fd: i32) {
    let mut scratch = [0u8; 4096];
    loop {
        // SAFETY: a borrowed descriptor and a live buffer. A short read or an error both end this.
        let n = unsafe { nix::libc::read(fd, scratch.as_mut_ptr().cast(), scratch.len()) };
        if n <= 0 {
            return;
        }
        if (n as usize) < scratch.len() {
            return;
        }
    }
}

/// How long the idle wait may block, in milliseconds — `None` for "until something happens".
///
/// Installed by whoever owns the timers. The editor asks before every wait, so a timer set while
/// sitting at a prompt shortens the *next* wait rather than the one already running; that costs at
/// most one keystroke of lateness and avoids having to interrupt a wait to re-arm it.
static DEADLINE: OnceLock<Box<dyn Fn() -> Option<u64> + Send + Sync>> = OnceLock::new();

/// Say how the nearest deadline is found.
pub fn install_deadline(deadline: impl Fn() -> Option<u64> + Send + Sync + 'static) {
    let _ = DEADLINE.set(Box::new(deadline));
}

/// The timeout for one wait: milliseconds, or `-1` for no timeout at all.
///
/// Clamped to `i32`, which is `poll`'s argument. A deadline further away than twenty-four days is
/// the same as no deadline for anybody's purposes, and saturating is better than wrapping into a
/// negative — which `poll` reads as "wait for ever" and would make a timer never fire.
pub fn wait_ms() -> i32 {
    match DEADLINE.get().and_then(|deadline| deadline()) {
        Some(ms) => i32::try_from(ms).unwrap_or(i32::MAX),
        None => -1,
    }
}

/// Wake the idle wait from another thread.
///
/// **For a worker that has no signal to send.** `oslo.spawn` finishes on a thread, appends its
/// result, and calls this; the editor's wait returns and the result is delivered where Lua may be
/// called. One byte, and it does not matter if the pipe is already full — a full pipe means a wake
/// is already pending, which is the same message.
pub fn nudge() {
    let Some((_, write)) = pipe() else { return };
    let byte = [0u8; 1];
    // SAFETY: a borrowed descriptor and one live byte. `EAGAIN` on a full pipe is the success case.
    unsafe {
        nix::libc::write(write, byte.as_ptr().cast(), 1);
    }
}

/// Make the self-pipe now, so it is in the waker set before the first wait.
///
/// **Not left to the first [`nudge`], and this was a real bug.** Creating it lazily meant the first
/// worker to finish registered a descriptor the editor's *current* `poll` was not watching — so the
/// wake went into a pipe nobody was listening to and the callback waited for a keystroke after all.
/// Exactly the case the nudge exists to remove.
pub fn arm() {
    if let Some((_, write)) = pipe() {
        NUDGE_FD.store(write, std::sync::atomic::Ordering::Relaxed);
    }
}

/// The write end, as one word a signal handler may read.
///
/// **Because [`nudge`] is not safe to call from a handler and this is.** `nudge` reaches a
/// `OnceLock`, which on its first call takes a lock — and a handler that blocks on a lock its own
/// interrupted thread holds is a hang with no way out. One relaxed load and one `write(2)`, both
/// async-signal-safe, is the whole of what a handler may do.
static NUDGE_FD: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1);

/// Wake the idle wait from a signal handler.
///
/// # Safety
///
/// Async-signal-safe: a relaxed atomic load and one `write` of a single byte, with the result
/// discarded. `EAGAIN` on a full pipe is the success case — a full pipe means a wake is already
/// pending, which is the same message.
pub fn nudge_from_signal() {
    let fd = NUDGE_FD.load(std::sync::atomic::Ordering::Relaxed);
    if fd < 0 {
        return;
    }
    let byte = [0u8; 1];
    // SAFETY: `fd` is the pipe's write end, open for the life of the process, and the buffer is one
    // live byte on this frame.
    unsafe {
        nix::libc::write(fd, byte.as_ptr().cast(), 1);
    }
}

/// The self-pipe, made once and registered as a waker the first time anybody wants it.
fn pipe() -> Option<(i32, i32)> {
    static PIPE: OnceLock<Option<(i32, i32)>> = OnceLock::new();
    *PIPE.get_or_init(|| {
        let mut ends = [0i32; 2];
        // SAFETY: a live array of two descriptors, which is what `pipe2` fills.
        let made = unsafe {
            nix::libc::pipe2(
                ends.as_mut_ptr(),
                nix::libc::O_CLOEXEC | nix::libc::O_NONBLOCK,
            )
        };
        if made != 0 {
            return None;
        }
        wake_on(ends[0]);
        Some((ends[0], ends[1]))
    })
}

/// Somewhere for a watch to keep the thing it must not drop.
///
/// A registered descriptor has to outlive the shell's interest in it — see [`wake_on`] — and the
/// owner is usually a local that would otherwise fall out of scope at the end of startup.
static KEPT: Mutex<Vec<Box<dyn std::any::Any + Send>>> = Mutex::new(Vec::new());

/// Keep `owner` alive for the rest of the process.
pub fn keep(owner: impl std::any::Any + Send + 'static) {
    if let Ok(mut kept) = KEPT.lock() {
        kept.push(Box::new(owner));
    }
}

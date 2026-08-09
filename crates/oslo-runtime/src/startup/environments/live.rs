//! Showing an `.envrc` working, while it is still working.
//!
//! Its output is captured to a scratch file so that it can be printed under the line naming the
//! file — see [`super::capturing`]. That is right for a file that takes a moment and wrong for one
//! that takes a minute: `use flake` on a cold store fetches and builds for as long as it takes, and
//! every word of it lands in a file nobody is watching. The shell prints nothing, the prompt does
//! not come back, and there is no way to tell a slow build from a hung one.
//!
//! So the scratch file is *tailed* while the rc file runs, and whatever has arrived is written to
//! the real terminal in the same shape the block uses. What it says is what `direnv` would have
//! shown; the difference is that it is drawn in oslo's own rail rather than left raw.
//!
//! **Nothing is shown for a file that is quick.** Printing starts only after [`QUIET`] has passed
//! with the rc file still running, so the overwhelming majority of arrivals — an `.envrc` that
//! exports four variables and returns — look exactly as they did: one block, nothing before it.
//!
//! **Lines are printed once.** The tail counts the bytes it has put on screen and hands the number
//! back, so the summary that follows prints only what is left.

use oslo_ui::block::Block;
use std::io::{Read, Seek};
use std::os::fd::RawFd;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

/// How long a rc file may run before anything it says is worth showing.
///
/// Under this and it would be a flicker between a `cd` and a prompt; over it and the alternative is
/// a shell that looks stuck. Half a second is roughly where "it did not print" turns into "is it
/// broken".
const QUIET: Duration = Duration::from_millis(500);

/// How often the scratch file is checked once printing has started.
const EVERY: Duration = Duration::from_millis(80);

/// A tail running behind a rc file, and what it has already shown.
pub(super) struct Live {
    stop: Arc<AtomicBool>,
    printed: Arc<AtomicUsize>,
    tail: Option<std::thread::JoinHandle<()>>,
}

impl Live {
    /// Start tailing `scratch`, writing to `terminal`.
    ///
    /// `terminal` is the caller's *saved* stdout, taken before the redirect: writing to the
    /// ordinary one would put the progress back into the file it is being read from. It stays open
    /// until after [`Self::stop`], which is why that must be called before the caller reclaims it.
    ///
    /// `columns` is measured by the caller for the same reason — asking here would ask the scratch
    /// file how wide it is and get the fallback.
    pub(super) fn start(scratch: &std::fs::File, terminal: RawFd, columns: usize) -> Live {
        let stop = Arc::new(AtomicBool::new(false));
        let printed = Arc::new(AtomicUsize::new(0));
        let Ok(mut reading) = scratch.try_clone() else {
            // Without a second handle there is nothing to tail from; the summary still prints
            // everything at the end, which is the behaviour this improves on rather than replaces.
            return Live {
                stop,
                printed,
                tail: None,
            };
        };
        let (flag, counter) = (Arc::clone(&stop), Arc::clone(&printed));
        let tail = std::thread::spawn(move || {
            if waited_out(&flag) {
                return;
            }
            let mut at = 0u64;
            let mut pending = String::new();
            while !flag.load(Ordering::Relaxed) {
                at = drain(&mut reading, at, &mut pending, terminal, columns, &counter);
                std::thread::sleep(EVERY);
            }
            // One last look: the rc file may have written its final lines between the sleep and
            // the flag being set, and they are worth showing in the order they happened.
            drain(&mut reading, at, &mut pending, terminal, columns, &counter);
        });
        Live {
            stop,
            printed,
            tail: Some(tail),
        }
    }

    /// Stop tailing and answer how many bytes reached the screen.
    pub(super) fn stop(self) -> usize {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(tail) = self.tail {
            let _ = tail.join();
        }
        self.printed.load(Ordering::Relaxed)
    }
}

/// Sleep out [`QUIET`], answering whether the rc file finished inside it.
///
/// Checked in slices rather than slept through, so a file that finishes in 20ms does not keep a
/// thread alive for the rest of the second.
fn waited_out(flag: &AtomicBool) -> bool {
    let mut waited = Duration::ZERO;
    while waited < QUIET {
        if flag.load(Ordering::Relaxed) {
            return true;
        }
        std::thread::sleep(EVERY);
        waited += EVERY;
    }
    false
}

/// Print whatever whole lines have arrived since `at`, and answer the new offset.
///
/// A partial line is held back in `pending` rather than drawn: a build tool writes its progress a
/// piece at a time, and half a path on the screen is worse than the same path a moment later. What
/// is held back is never counted as printed, so the summary picks it up.
fn drain(
    reading: &mut std::fs::File,
    at: u64,
    pending: &mut String,
    terminal: RawFd,
    columns: usize,
    counter: &AtomicUsize,
) -> u64 {
    let Ok(_) = reading.seek(std::io::SeekFrom::Start(at)) else {
        return at;
    };
    let mut fresh = Vec::new();
    if reading.read_to_end(&mut fresh).is_err() || fresh.is_empty() {
        return at;
    }
    let moved = at + fresh.len() as u64;
    pending.push_str(&String::from_utf8_lossy(&fresh));

    while let Some(end) = pending.find('\n') {
        let line: String = pending.drain(..=end).collect();
        let text = line.trim_end();
        counter.fetch_add(line.len(), Ordering::Relaxed);
        if text.is_empty() {
            continue;
        }
        write_line(terminal, text, columns);
    }
    moved
}

/// One line, in the block's own rail, written straight to the terminal.
fn write_line(terminal: RawFd, text: &str, columns: usize) {
    // The shell's own errors already say `oslo:`; the rail says which file is talking.
    let text = text.strip_prefix("oslo: ").unwrap_or(text);
    let mut block = Block::new("").width(columns);
    block.note(text);
    let mut drawn = block.lines().join("\n");
    drawn.push('\n');
    // SAFETY: `terminal` is the caller's saved stdout, open until `Live::stop` returns.
    let fd = unsafe { std::os::fd::BorrowedFd::borrow_raw(terminal) };
    let _ = nix::unistd::write(fd, drawn.as_bytes());
}

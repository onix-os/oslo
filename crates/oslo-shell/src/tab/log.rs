//! Everything the tab printed while nobody was watching, up to a cap.
//!
//! The case this whole feature exists for is a build you walked away from, so the output it produced
//! in your absence is the point rather than a nicety. It goes to a file so that `tail -f` works
//! without attaching, and so that a tab which has since exited still has something to read.
//!
//! # Why it is capped, and capped by rewriting
//!
//! The file lives under `/tmp`, which is **tmpfs — memory**. An uncapped log is a job that can
//! spend the machine's RAM by being chatty, so there is a ceiling and the oldest bytes lose.
//!
//! Trimming rewrites the file in place rather than rotating: one path stays one path, so a
//! `tail -f` that is already running keeps following the same inode instead of watching a file that
//! silently stopped being the one it wanted. The cost is copying the kept half, which happens once
//! per cap's worth of output and never in the middle of a burst — the check is on a whole write,
//! not on a byte.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

/// How much of the cap survives a trim.
///
/// Half rather than all of it: trimming to exactly the cap would trim again on the next write, and
/// the copy would be per-write rather than per-cap.
const KEEP: f64 = 0.5;

pub struct Log {
    file: File,
    cap: u64,
    size: u64,
}

impl Log {
    /// Open (or create) the log at `path`, appending to whatever is there.
    pub fn open(path: &Path, cap: u64) -> io::Result<Log> {
        // Read as well as append: the trim below reads the tail back before rewriting it. `.write`
        // is not asked for — `append` already implies it, and clippy is right that saying both
        // reads as though one of them did something.
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .append(true)
            .open(path)?;
        let size = file.metadata()?.len();
        Ok(Log { file, cap, size })
    }

    /// Append, and trim if this write took it over the cap.
    pub fn append(&mut self, bytes: &[u8]) -> io::Result<()> {
        if bytes.is_empty() {
            return Ok(());
        }
        self.file.write_all(bytes)?;
        self.size += bytes.len() as u64;
        if self.cap > 0 && self.size > self.cap {
            self.trim()?;
        }
        Ok(())
    }

    /// Keep the newest `cap * KEEP` bytes and drop the rest.
    ///
    /// A cap smaller than one write is honoured by keeping the tail of that write, which is the only
    /// answer that does not either lie about the cap or throw away the newest output.
    fn trim(&mut self) -> io::Result<()> {
        let keep = ((self.cap as f64) * KEEP).max(1.0) as u64;
        let from = self.size.saturating_sub(keep);
        let mut kept = Vec::with_capacity(keep as usize);
        self.file.seek(SeekFrom::Start(from))?;
        self.file.read_to_end(&mut kept)?;

        self.file.set_len(0)?;
        // An appending descriptor ignores the seek for *writes*, but the read above moved it and
        // `set_len` does not; rewinding keeps the two views of the file agreeing.
        self.file.seek(SeekFrom::Start(0))?;
        self.file.write_all(&kept)?;
        self.size = kept.len() as u64;
        Ok(())
    }
}

/// The last `bytes` of the log, for redrawing a pane on attach.
///
/// Not the whole file: a screen is what a person needs to see where they were, and replaying a
/// megabyte would take longer than the thing that produced it.
pub fn tail(path: &Path, bytes: u64) -> io::Result<Vec<u8>> {
    let mut file = File::open(path)?;
    let size = file.metadata()?.len();
    let from = size.saturating_sub(bytes);
    file.seek(SeekFrom::Start(from))?;
    let mut out = Vec::new();
    file.read_to_end(&mut out)?;
    Ok(out)
}

#[cfg(test)]
mod tests;

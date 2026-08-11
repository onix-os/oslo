//! The four files a tab is made of, and how to tell a live one from a leftover.
//!
//! ```text
//! <dir>/alpha.lock   the keeper holds an exclusive flock on this for its whole life
//! <dir>/alpha.sock   attach by connecting
//! <dir>/alpha.meta   what it is: where it started, when, and under what pid
//! <dir>/alpha.log    capped scrollback
//! ```
//!
//! # Liveness is a lock, not a pid
//!
//! A pid can be reused, and a pid file has to be cleaned up by whoever wrote it — which is exactly
//! what a process killed with `-9` cannot do. An `flock` is held by the *open file description*, so
//! the kernel drops it when the holder dies however it dies. Asking whether a tab is alive is then
//! asking whether the lock is taken: **if it can be locked, nobody is behind it**, and the leftovers
//! can be swept by whoever noticed.
//!
//! That is the whole of the registry. There is no daemon, and listing tabs is `readdir`.

use nix::fcntl::{Flock, FlockArg};
use std::fs::{File, OpenOptions};
use std::io;
use std::path::PathBuf;
use std::time::SystemTime;

/// One tab's files.
pub struct Paths {
    pub name: String,
}

impl Paths {
    pub fn new(name: &str) -> Paths {
        Paths {
            name: name.to_string(),
        }
    }

    fn with(&self, extension: &str) -> PathBuf {
        super::dir::path().join(format!("{}.{extension}", self.name))
    }

    pub fn lock(&self) -> PathBuf {
        self.with("lock")
    }
    pub fn sock(&self) -> PathBuf {
        self.with("sock")
    }
    pub fn meta(&self) -> PathBuf {
        self.with("meta")
    }
    pub fn log(&self) -> PathBuf {
        self.with("log")
    }
}

/// What a tab says about itself. Written once by the keeper, read by anything listing.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Meta {
    pub cwd: String,
    /// Seconds since the epoch.
    pub started: u64,
    pub pid: i32,
}

impl Meta {
    /// `key=value` a line at a time — the same shape `.meta` has had since it was one line long,
    /// and readable with `cat` when something has gone wrong.
    pub fn encode(&self) -> String {
        format!(
            "cwd={}\nstarted={}\npid={}\n",
            self.cwd, self.started, self.pid
        )
    }

    /// Anything unparseable is a default rather than an error: a tab with a torn `.meta` is still a
    /// tab you want listed, and the fields are decoration.
    pub fn decode(text: &str) -> Meta {
        let mut meta = Meta::default();
        for line in text.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            match key {
                "cwd" => meta.cwd = value.to_string(),
                "started" => meta.started = value.parse().unwrap_or(0),
                "pid" => meta.pid = value.parse().unwrap_or(0),
                _ => {}
            }
        }
        meta
    }

    pub fn now(cwd: &str) -> Meta {
        Meta {
            cwd: cwd.to_string(),
            started: SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .map(|since| since.as_secs())
                .unwrap_or(0),
            pid: std::process::id() as i32,
        }
    }
}

/// Take the tab's lock for as long as the returned guard lives.
///
/// The keeper calls this once and holds it until it dies. `None` means somebody else has it, which
/// is the same sentence as "that tab is already running".
pub fn hold(name: &str) -> io::Result<Option<Flock<File>>> {
    let path = Paths::new(name).lock();
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&path)?;
    match Flock::lock(file, FlockArg::LockExclusiveNonblock) {
        Ok(held) => Ok(Some(held)),
        Err((_, nix::errno::Errno::EWOULDBLOCK)) => Ok(None),
        Err((_, err)) => Err(io::Error::other(format!("{}: {err}", path.display()))),
    }
}

/// Whether anything is behind this name.
///
/// Implemented by trying to take the lock and giving it straight back: the attempt is the question.
pub fn alive(name: &str) -> bool {
    match hold(name) {
        // We got it, so nobody was holding it. Dropping the guard releases it again.
        Ok(Some(_)) => false,
        Ok(None) => true,
        // A lock we cannot even open is not a tab we can attach to, so it is not alive.
        Err(_) => false,
    }
}

/// Every tab in the directory, live ones first, newest first.
///
/// Sweeps as it goes: a name whose lock is free has no keeper, so its files are leftovers from a
/// keeper that was killed rather than asked to stop, and this is the only thing that will ever
/// clean them up.
pub fn list() -> io::Result<Vec<(String, Meta)>> {
    let dir = super::dir::path();
    let mut found = Vec::new();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Ok(found);
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("lock") {
            continue;
        }
        let Some(name) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        if !super::name::valid(name) {
            continue;
        }
        if alive(name) {
            let meta = std::fs::read_to_string(Paths::new(name).meta())
                .map(|text| Meta::decode(&text))
                .unwrap_or_default();
            found.push((name.to_string(), meta));
        } else {
            sweep(name);
        }
    }
    found.sort_by(|a, b| b.1.started.cmp(&a.1.started).then_with(|| a.0.cmp(&b.0)));
    Ok(found)
}

/// Move a tab's files to a new name.
///
/// The socket keeps working: renaming the file does not touch the listening descriptor, and a
/// client connecting to the new path reaches the same inode. The lock follows for the same reason —
/// an `flock` belongs to the open file description, not to the path it was opened through.
pub fn rename(from: &str, to: &str) {
    let (old, new) = (Paths::new(from), Paths::new(to));
    for (a, b) in [
        (old.sock(), new.sock()),
        (old.meta(), new.meta()),
        (old.log(), new.log()),
        (old.lock(), new.lock()),
    ] {
        let _ = std::fs::rename(a, b);
    }
}

/// Remove what a dead tab left behind. Best effort by nature — this is tidying, not bookkeeping.
pub fn sweep(name: &str) {
    let paths = Paths::new(name);
    for path in [paths.sock(), paths.meta(), paths.log(), paths.lock()] {
        let _ = std::fs::remove_file(path);
    }
}

/// The names in use, for [`super::name::suggest`].
pub fn taken() -> Vec<String> {
    list()
        .unwrap_or_default()
        .into_iter()
        .map(|(name, _)| name)
        .collect()
}

#[cfg(test)]
mod tests;

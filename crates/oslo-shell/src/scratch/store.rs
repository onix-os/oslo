//! The four files a scratch is made of, and how to tell a live one from a leftover.
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
//! the kernel drops it when the holder dies however it dies. Asking whether a scratch is alive is then
//! asking whether the lock is taken: **if it can be locked, nobody is behind it**, and the leftovers
//! can be swept by whoever noticed.
//!
//! That is the whole of the registry. There is no daemon, and listing scratches is `readdir`.

use nix::fcntl::{Flock, FlockArg};
use std::fs::{File, OpenOptions};
use std::io;
use std::path::PathBuf;
use std::time::SystemTime;

/// One scratch's files.
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

/// What a scratch says about itself. Written once by the keeper, read by anything listing.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Meta {
    pub cwd: String,
    /// Seconds since the epoch.
    pub started: u64,
    /// The shell inside. Ending a scratch is ending this.
    pub pid: i32,
    /// The keeper holding the pty, for when the shell will not go.
    pub keeper: i32,
}

impl Meta {
    /// `key=value` a line at a time — the same shape `.meta` has had since it was one line long,
    /// and readable with `cat` when something has gone wrong.
    pub fn encode(&self) -> String {
        format!(
            "cwd={}\nstarted={}\npid={}\nkeeper={}\n",
            self.cwd, self.started, self.pid, self.keeper
        )
    }

    /// Anything unparseable is a default rather than an error: a scratch with a torn `.meta` is still a
    /// scratch you want listed, and the fields are decoration.
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
                "keeper" => meta.keeper = value.parse().unwrap_or(0),
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
            keeper: std::process::id() as i32,
        }
    }
}

/// Take the scratch's lock for as long as the returned guard lives.
///
/// The keeper calls this once and holds it until it dies. `None` means somebody else has it, which
/// is the same sentence as "that scratch is already running".
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
        // A lock we cannot even open is not a scratch we can attach to, so it is not alive.
        Err(_) => false,
    }
}

/// Every scratch in the directory, live ones first, newest first.
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

/// End the scratch behind `name`, and clean up after it.
///
/// # The shell first, and only then the keeper
///
/// A scratch is over when the shell inside it exits — that is what `exit` has always meant, and
/// hanging up on the shell is the same ending arriving from outside. The keeper then sees EOF on
/// the pty, sweeps its own files and goes, exactly as it does on an ordinary `exit`. Killing the
/// keeper first would work too, and would leave the ending to be tidied by whoever next listed.
///
/// # Why these pids can be trusted
///
/// A pid file is usually a lie waiting to happen, because the process it names can be gone and its
/// number reused. Not here. **The lock proves the keeper is alive**, and the keeper never reaps the
/// shell it forked — so while `alive` says yes, that pid is either the shell or a zombie of it, and
/// the kernel will not hand the number to anybody else. Everything below is guarded by that.
pub fn kill(name: &str) -> io::Result<()> {
    use nix::sys::signal::{Signal, kill as signal};
    use nix::unistd::Pid;

    if !alive(name) {
        // Nothing is holding it, so this is a name with leftovers rather than a scratch.
        sweep(name);
        return Ok(());
    }
    let paths = Paths::new(name);
    let meta = std::fs::read_to_string(paths.meta())
        .map(|text| Meta::decode(&text))
        .unwrap_or_default();

    for (pid, sig) in [
        (meta.pid, Signal::SIGHUP),
        (meta.pid, Signal::SIGKILL),
        (meta.keeper, Signal::SIGKILL),
    ] {
        if pid <= 0 {
            continue;
        }
        let _ = signal(Pid::from_raw(pid), sig);
        if gone(name) {
            // The keeper sweeps after itself when it is the one that noticed.
            sweep(name);
            return Ok(());
        }
    }
    Err(io::Error::other(format!("{name} would not stop")))
}

/// Wait a moment for the lock to come free, which is the scratch being over.
fn gone(name: &str) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        if !alive(name) {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    false
}

/// Remove what a dead scratch left behind. Best effort by nature — this is tidying, not bookkeeping.
pub fn sweep(name: &str) {
    let paths = Paths::new(name);
    for path in [paths.sock(), paths.meta(), paths.log(), paths.lock()] {
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests;

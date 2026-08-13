//! Running an external program from inside a builtin.
//!
//! `command` and `exec` both have to reach a binary on `PATH` without going back through
//! [`crate::exec::simple`], whose whole job is the lookup order those two builtins exist to
//! bypass. The fork/exec mechanics are the same either way, so they live here once.
//!
//! Redirections are *not* applied here. A builtin runs with the shell's descriptors already
//! pointing where the command's redirections say (`crate::exec::redirect::RedirectGuard` is
//! applied by the caller before the builtin is entered), and a forked child inherits them.

use nix::sys::wait::{WaitPidFlag, WaitStatus, waitpid};
use nix::unistd::{ForkResult, Pid, fork};
use oslo_base::error::{Result, ShellError};
use std::ffi::CString;
use std::path::{Path, PathBuf};

/// Turn user-controlled bytes into an argv entry, dropping any NUL.
///
/// argv entries are NUL-terminated, so an embedded NUL cannot reach `execv` under any encoding.
/// Dropping it reproduces the argument bash would have built; the one thing this must never do is
/// panic, which is what a `CString::new(..).unwrap()` on shell data would eventually do.
pub fn exec_cstring(bytes: &[u8]) -> CString {
    let stripped: Vec<u8> = bytes.iter().copied().filter(|b| *b != 0).collect();
    CString::new(stripped).unwrap_or_default()
}

/// Resolve a command word to a program on disk the way the shell does.
///
/// A word containing a slash is a path and is used as given — `PATH` is not consulted, and a
/// relative path stays relative to the working directory. Anything else is searched for on `PATH`.
pub fn resolve_program(name: &str) -> Option<PathBuf> {
    if name.contains('/') {
        let path = Path::new(name);
        return path.is_file().then(|| path.to_path_buf());
    }
    // **oslo does not see its own copies.** `macros::bin` writes every stored script into a
    // directory on `$PATH` so that bash, tmux and a `.desktop` file can run one; oslo has the
    // database and needs no copy, so that directory is taken *out of the search* and the macro is
    // answered from the database instead — after the rest of `$PATH`, which is where a stored macro
    // belongs.
    //
    // **Out of the search, not out of the answer.** Rejecting the path `which` came back with was
    // the first attempt and it is wrong in a way that matters: it ends the search, so a stored
    // `date` with a copy early on `$PATH` beat `/usr/bin/date` — exactly the shadowing this whole
    // design promises cannot happen. The directory is removed from `$PATH` and the rest is walked.
    match without_our_copies() {
        Some(path) => which::which_in(name, Some(path), std::env::current_dir().ok()?).ok(),
        None => which::which(name).ok(),
    }
}

/// `$PATH` without the directory oslo writes its own scripts into, or `None` when it is not there.
fn without_our_copies() -> Option<std::ffi::OsString> {
    let ours = oslo_base::macros::bin::directory()?;
    let path = std::env::var_os("PATH")?;
    let kept: Vec<PathBuf> = std::env::split_paths(&path)
        .filter(|entry| entry != &ours)
        .collect();
    // Unchanged means the copies are not on `$PATH` at all, and the ordinary search is the cheaper
    // answer — this runs on every command that is not a builtin.
    (kept.len() != std::env::split_paths(&path).count())
        .then(|| std::env::join_paths(kept).ok())
        .flatten()
}

/// The status a shell reports when a command word could not be run at all.
///
/// 127 for "no such command", 126 for "found it and could not execute it" — the split every
/// script's `if [ $? -eq 127 ]` depends on.
pub const NOT_FOUND: i32 = 127;
pub const NOT_EXECUTABLE: i32 = 126;

/// Fork, exec `program` with `argv`, and wait for it.
///
/// `argv` is the full argument vector including argv\[0\]; the caller decides what argv\[0\] is,
/// which is what lets `command foo` keep reporting itself as `foo`.
pub fn run_external(program: &Path, argv: &[String], display_name: &str) -> Result<i32> {
    let c_path = exec_cstring(std::os::unix::ffi::OsStrExt::as_bytes(program.as_os_str()));
    let c_args: Vec<CString> = argv.iter().map(|a| exec_cstring(a.as_bytes())).collect();

    // SAFETY: the child touches only async-signal-safe calls (`sigaction`, `sigprocmask`,
    // `execv`, `write` via `eprintln`, `_exit`) before it replaces itself. oslo is single
    // threaded, so no other thread's lock can be inherited half-held.
    match unsafe { fork() } {
        Ok(ForkResult::Child) => {
            crate::exec::job::reset_signals_for_child();
            let _ = nix::unistd::execv(&c_path, &c_args);
            eprintln!("oslo: {}: cannot execute", display_name);
            std::process::exit(NOT_EXECUTABLE);
        }
        Ok(ForkResult::Parent { child }) => Ok(wait_for_child(child)),
        Err(e) => Err(ShellError::ExecutionError(format!("Fork failed: {}", e))),
    }
}

/// Wait for a foreground child and turn its wait status into an exit status.
///
/// Mirrors [`crate::exec::simple`]: a signal death is `128 + signo`, a stop is reported rather
/// than waited on forever, and `EINTR` — a trapped signal arriving mid-wait — says nothing about
/// how the command ended, so it is retried.
fn wait_for_child(child: Pid) -> i32 {
    loop {
        match waitpid(child, Some(WaitPidFlag::WUNTRACED)) {
            Ok(WaitStatus::Exited(_, code)) => return code,
            Ok(WaitStatus::Signaled(_, sig, _)) => return 128 + sig as i32,
            Ok(WaitStatus::Stopped(_, sig)) => return 128 + sig as i32,
            Ok(_) => continue,
            Err(nix::errno::Errno::EINTR) => continue,
            Err(_) => return 1,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{exec_cstring, resolve_program};
    use std::ffi::CString;

    #[test]
    fn an_embedded_nul_is_dropped_not_fatal() {
        assert_eq!(exec_cstring(b"a\0b"), CString::new("ab").unwrap());
        assert_eq!(exec_cstring(b"plain"), CString::new("plain").unwrap());
    }

    /// A word with a slash is a path, not a `PATH` search — `./ls` must never find `/bin/ls`.
    #[test]
    fn a_slash_bearing_word_is_taken_as_a_path() {
        assert_eq!(
            resolve_program("/bin/sh").as_deref(),
            Some("/bin/sh".as_ref())
        );
        assert_eq!(resolve_program("./definitely-not-here-xyz"), None);
    }

    #[test]
    fn a_bare_word_is_searched_for_on_path() {
        assert!(resolve_program("sh").is_some());
        assert_eq!(resolve_program("definitely-not-a-command-xyz"), None);
    }
}

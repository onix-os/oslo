use crate::ast::{RedirectKind, Redirection};
use crate::env::Environment;
use crate::error::{Result, ShellError};
use crate::expand::expand_word;
use nix::fcntl::{FcntlArg, fcntl};
use nix::unistd::dup2;
use std::fs::{File, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::os::fd::{AsRawFd, RawFd};

/// Lowest descriptor number a saved copy may occupy.
///
/// bash reserves everything from 10 up for its own bookkeeping and so does this shell: a script
/// that writes `exec 3>log` or `cmd 2>&5` is entitled to descriptors 3..9, and a saved copy parked
/// there would be visible to — and clobberable by — the script that the save exists to protect.
const SAVE_FD_FLOOR: RawFd = 10;

/// Restores the descriptors a set of redirections overwrote, when it is dropped.
///
/// Each entry is the descriptor that was redirected plus, if it was open beforehand, a private
/// copy of what it used to point at. `None` means the descriptor was *closed* before the
/// redirection, and the only faithful restore is to close it again.
pub struct RedirectGuard {
    saved_fds: Vec<(RawFd, Option<RawFd>)>,
    /// When false, `apply` skips saving entirely and `Drop` has nothing to undo.
    save: bool,
}

impl Default for RedirectGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl RedirectGuard {
    pub fn new() -> Self {
        Self {
            saved_fds: Vec::new(),
            save: true,
        }
    }

    /// A guard for a forked child that is about to `execv`, where nothing is ever restored.
    ///
    /// The child's descriptor table dies with the `exec`, so saving copies of it only burns
    /// syscalls — and, if the shell ever stops calling `std::process::exit` on the failure path,
    /// risks handing the new program descriptors it was never meant to see.
    pub fn for_exec() -> Self {
        Self {
            saved_fds: Vec::new(),
            save: false,
        }
    }

    pub fn apply(&mut self, env: &mut Environment, redirections: &[Redirection]) -> Result<()> {
        for redir in redirections {
            let target_words = expand_word(env, &redir.target)?;
            let target_str = target_words.join(" ");

            let target_fd = redir.fd.unwrap_or(match redir.kind {
                RedirectKind::Input
                | RedirectKind::DupInput
                | RedirectKind::Heredoc
                | RedirectKind::HeredocStrip
                | RedirectKind::ReadWrite => 0,
                _ => 1,
            });

            if self.save {
                self.saved_fds.push((target_fd, save_fd(target_fd)));
            }

            match redir.kind {
                RedirectKind::Input => {
                    let file = File::open(&target_str).map_err(|e| {
                        ShellError::ExecutionError(format!("{}: {}", target_str, e))
                    })?;
                    dup2(file.as_raw_fd(), target_fd)?;
                }
                // `>` and `>|` differ in exactly one situation, and only when `set -C` is on.
                RedirectKind::Output => {
                    let file = open_for_output(&target_str, env.noclobber())?;
                    dup2(file.as_raw_fd(), target_fd)?;
                }
                RedirectKind::Clobber => {
                    let file = open_for_output(&target_str, false)?;
                    dup2(file.as_raw_fd(), target_fd)?;
                }
                RedirectKind::Append => {
                    let file = OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&target_str)
                        .map_err(|e| {
                            ShellError::ExecutionError(format!("{}: {}", target_str, e))
                        })?;
                    dup2(file.as_raw_fd(), target_fd)?;
                }
                RedirectKind::ReadWrite => {
                    let file = OpenOptions::new()
                        .read(true)
                        .write(true)
                        .create(true)
                        .truncate(false)
                        .open(&target_str)
                        .map_err(|e| {
                            ShellError::ExecutionError(format!("{}: {}", target_str, e))
                        })?;
                    dup2(file.as_raw_fd(), target_fd)?;
                }
                RedirectKind::DupInput | RedirectKind::DupOutput => {
                    if target_str == "-" {
                        let _ = nix::unistd::close(target_fd);
                    } else if let Ok(src_fd) = target_str.parse::<RawFd>() {
                        // Report the descriptor number the script named, the way bash does; the
                        // raw errno text ("EBADF: Bad file descriptor") names nothing the author
                        // can act on.
                        dup2(src_fd, target_fd).map_err(|_| {
                            ShellError::ExecutionError(format!(
                                "{}: Bad file descriptor",
                                target_str
                            ))
                        })?;
                    } else {
                        return Err(ShellError::ExecutionError(format!(
                            "Invalid file descriptor for dup: {}",
                            target_str
                        )));
                    }
                }
                RedirectKind::Heredoc | RedirectKind::HeredocStrip => {
                    let content = redir.heredoc_content.as_deref().unwrap_or("");
                    let body = heredoc_body(content)?;
                    dup2(body.as_raw_fd(), target_fd)?;
                }
            }
        }
        Ok(())
    }
}

/// Open the target of `>` or `>|`, honouring `set -C` when `refuse_existing` says to.
///
/// With noclobber off this is a plain create-or-truncate. With it on, `>` must not destroy a file
/// that is already there — the whole point of the option is that a mistyped `> notes` cannot eat
/// the file. `create_new` is what enforces it, rather than a `Path::exists` test followed by an
/// open: between the test and the open another process can create the file, and the check that
/// loses that race silently truncates exactly the data the option exists to protect.
///
/// Two things noclobber deliberately does *not* refuse:
///
/// * anything that is not a regular file. `cmd > /dev/null` and `cmd > /dev/tty` have nothing to
///   overwrite, and POSIX scopes the restriction to regular files for that reason.
/// * `>|`, which is the escape hatch — see the caller.
fn open_for_output(path: &str, refuse_existing: bool) -> Result<File> {
    if !refuse_existing {
        return OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)
            .map_err(|e| ShellError::ExecutionError(format!("{}: {}", path, e)));
    }

    match OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(file) => Ok(file),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            // `symlink_metadata` is wrong here: `> link-to-dev-null` follows the link in every
            // shell, so the question is what the *target* is.
            let regular = std::fs::metadata(path).is_ok_and(|m| m.is_file());
            if regular {
                return Err(ShellError::ExecutionError(format!(
                    "{}: cannot overwrite existing file",
                    path
                )));
            }
            OpenOptions::new()
                .write(true)
                .open(path)
                .map_err(|e| ShellError::ExecutionError(format!("{}: {}", path, e)))
        }
        Err(e) => Err(ShellError::ExecutionError(format!("{}: {}", path, e))),
    }
}

/// Materialise a heredoc or here-string body as a readable fd positioned at byte 0.
///
/// The body must not go into a pipe. Nothing is reading the far end yet — the command that will
/// read it has not been forked — so a body larger than the kernel's pipe buffer (64 KB by default)
/// blocks the shell forever. In the REPL the `SA_RESTART` SIGINT handler makes that hang
/// un-interruptible. A seekable file has no capacity limit, so any body size works.
fn heredoc_body(content: &str) -> Result<File> {
    let mut file = anonymous_file()?;
    file.write_all(content.as_bytes())
        .map_err(|e| ShellError::ExecutionError(format!("heredoc: {}", e)))?;
    file.seek(SeekFrom::Start(0))
        .map_err(|e| ShellError::ExecutionError(format!("heredoc: {}", e)))?;
    Ok(file)
}

/// An empty read/write file with no name in the filesystem, so nothing has to clean it up.
fn anonymous_file() -> Result<File> {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        use nix::sys::memfd::{MemFdCreateFlag, memfd_create};
        // ENOSYS below Linux 3.17, and blocked by some seccomp policies; fall back rather than
        // making heredocs a kernel-version feature.
        if let Ok(fd) = memfd_create(c"rush-heredoc", MemFdCreateFlag::MFD_CLOEXEC) {
            return Ok(File::from(fd));
        }
    }
    unlinked_temp_file()
}

/// A temp file unlinked the instant it exists: the fd stays valid, the name does not.
fn unlinked_temp_file() -> Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    use std::sync::atomic::{AtomicU32, Ordering};

    static SEQ: AtomicU32 = AtomicU32::new(0);

    let dir = std::env::temp_dir();
    // A name collision is another process (or an earlier heredoc in this one) owning the path;
    // retry rather than clobbering it, since `create_new` guarantees we never open someone else's.
    for _ in 0..64 {
        let path = dir.join(format!(
            "rush-heredoc-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        match OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(nix::fcntl::OFlag::O_CLOEXEC.bits())
            .open(&path)
        {
            Ok(file) => {
                let _ = std::fs::remove_file(&path);
                return Ok(file);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => {
                return Err(ShellError::ExecutionError(format!(
                    "heredoc: {}: {}",
                    dir.display(),
                    e
                )));
            }
        }
    }
    Err(ShellError::ExecutionError(format!(
        "heredoc: {}: could not create a temporary file",
        dir.display()
    )))
}

/// Stash whatever `target_fd` currently points at, out of the way of the script and of `exec`.
///
/// `dup(2)` is the obvious call and the wrong one twice over. It hands back the *lowest* free
/// descriptor — 3 or 4 in a fresh shell — which puts the shell's private copy of stdout squarely
/// in the range a script addresses by number, so `2>&3` inside a redirected group silently
/// succeeds where it should fail. And a plain `dup` never sets `FD_CLOEXEC`, so every child the
/// shell forks inherits those copies. `F_DUPFD_CLOEXEC` with a floor fixes both.
///
/// `None` means there was nothing to save: `target_fd` was already closed, which is a state the
/// restore path has to be able to reproduce.
fn save_fd(target_fd: RawFd) -> Option<RawFd> {
    fcntl(target_fd, FcntlArg::F_DUPFD_CLOEXEC(SAVE_FD_FLOOR)).ok()
}

impl Drop for RedirectGuard {
    fn drop(&mut self) {
        // Reverse order: `cmd >a >b` saved the pre-`a` state first, and unwinding b-then-a is what
        // puts the descriptor back where the command found it.
        for (target_fd, saved_fd) in self.saved_fds.drain(..).rev() {
            match saved_fd {
                Some(saved_fd) => {
                    let _ = dup2(saved_fd, target_fd);
                    let _ = nix::unistd::close(saved_fd);
                }
                // Nothing was open here before, so restoring means closing. Skipping this is how
                // `cmd 5>file` used to leak descriptor 5 into every command the shell ran next.
                None => {
                    let _ = nix::unistd::close(target_fd);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::os::unix::fs::MetadataExt;

    /// On Linux `heredoc_body` always takes the memfd path, so the fallback is only ever reached
    /// on a kernel or sandbox this test suite does not run under. Exercise it directly.
    #[test]
    fn the_temp_file_fallback_is_readable_and_anonymous() {
        let mut file = unlinked_temp_file().expect("temp file");
        file.write_all(b"body").unwrap();
        file.seek(SeekFrom::Start(0)).unwrap();

        let mut got = String::new();
        file.read_to_string(&mut got).unwrap();
        assert_eq!(got, "body");

        // Unlinked at creation, so it leaves nothing behind even if the shell is killed.
        assert_eq!(file.metadata().unwrap().nlink(), 0);
    }

    #[test]
    fn a_body_larger_than_a_pipe_buffer_is_fully_readable() {
        let content = "y".repeat(1 << 20);
        let mut file = heredoc_body(&content).expect("heredoc body");

        let mut got = String::new();
        file.read_to_string(&mut got).unwrap();
        assert_eq!(got.len(), content.len());
    }

    /// The descriptor a script can name is 0..9. Anything the shell keeps for itself has to sit
    /// above that, or `2>&3` inside a redirected group hits the shell's own copy of stdout.
    #[test]
    fn a_saved_copy_lands_out_of_the_range_a_script_can_name() {
        let saved = save_fd(1).expect("stdout is open");
        assert!(
            saved >= SAVE_FD_FLOOR,
            "saved copy landed on fd {saved}, inside the range a script addresses by number"
        );
        let _ = nix::unistd::close(saved);
    }

    /// Without `FD_CLOEXEC` every child the shell forks inherits a duplicate of the shell's
    /// original stdout and stderr, and can read or scribble on them behind the redirection.
    #[test]
    fn a_saved_copy_does_not_survive_exec() {
        use nix::fcntl::FdFlag;

        let saved = save_fd(1).expect("stdout is open");
        let flags = FdFlag::from_bits_truncate(fcntl(saved, FcntlArg::F_GETFD).expect("F_GETFD"));
        assert!(
            flags.contains(FdFlag::FD_CLOEXEC),
            "saved copy on fd {saved} is not close-on-exec"
        );
        let _ = nix::unistd::close(saved);
    }

    /// A descriptor that was closed before the redirection has no copy to restore, and leaving it
    /// open is not "no restore" — it is a leak that outlives the command (`cmd 5>file`).
    #[test]
    fn a_descriptor_with_no_saved_copy_is_closed_on_restore() {
        // A private descriptor, not one of the shell's: these tests share a process, and a test
        // that reached for a fixed number like 5 would race whatever else is running.
        let opened = save_fd(1).expect("stdout is open");

        let mut guard = RedirectGuard::new();
        guard.saved_fds.push((opened, None));
        drop(guard);

        assert_eq!(
            fcntl(opened, FcntlArg::F_GETFD),
            Err(nix::errno::Errno::EBADF),
            "fd {opened} was left open with nothing to restore it to"
        );
    }

    /// `set -C` is the difference between `>` and `>|`, and it applies to regular files only.
    #[test]
    fn noclobber_refuses_only_an_existing_regular_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let fresh = dir.path().join("fresh");
        let fresh = fresh.to_str().unwrap();

        // Nothing there yet: `>` creates it even under noclobber.
        open_for_output(fresh, true).expect("creating a new file is always allowed");
        // Now it exists, so `>` must refuse and `>|` must not.
        let err = open_for_output(fresh, true).expect_err("noclobber must refuse");
        assert!(err.to_string().contains("cannot overwrite"), "{err}");
        open_for_output(fresh, false).expect("`>|` overrides noclobber");

        // Not a regular file, so there is nothing for the option to protect.
        open_for_output("/dev/null", true).expect("/dev/null is exempt");
    }

    /// The refusal must not be a read-then-open: the file has to be created by the same syscall
    /// that would have truncated it, or a racing creator's data is destroyed anyway.
    #[test]
    fn noclobber_truncates_nothing_when_it_refuses() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("data");
        std::fs::write(&path, b"payload").expect("write");
        let _ = open_for_output(path.to_str().unwrap(), true).expect_err("noclobber must refuse");
        assert_eq!(std::fs::read(&path).expect("read"), b"payload");
    }

    /// The pre-exec child never restores anything, so it must not spend syscalls building a save
    /// list — and must not leave copies of the shell's descriptors lying around either.
    #[test]
    fn the_exec_guard_saves_nothing() {
        let mut guard = RedirectGuard::for_exec();
        let mut env = Environment::new();
        guard.apply(&mut env, &[]).expect("no redirections");
        assert!(guard.saved_fds.is_empty());
    }
}

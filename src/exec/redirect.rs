use crate::ast::{RedirectKind, Redirection};
use crate::env::Environment;
use crate::error::{Result, ShellError};
use crate::expand::expand_word;
use nix::unistd::dup2;
use std::fs::{File, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::os::fd::{AsRawFd, IntoRawFd, RawFd};

pub struct RedirectGuard {
    saved_fds: Vec<(RawFd, RawFd)>,
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

            // Save target_fd if not saved already
            if let Ok(saved) = nix::unistd::dup(target_fd) {
                self.saved_fds.push((target_fd, saved.into_raw_fd()));
            }

            match redir.kind {
                RedirectKind::Input => {
                    let file = File::open(&target_str).map_err(|e| {
                        ShellError::ExecutionError(format!("{}: {}", target_str, e))
                    })?;
                    dup2(file.as_raw_fd(), target_fd)?;
                }
                RedirectKind::Output | RedirectKind::Clobber => {
                    let file = OpenOptions::new()
                        .write(true)
                        .create(true)
                        .truncate(true)
                        .open(&target_str)
                        .map_err(|e| {
                            ShellError::ExecutionError(format!("{}: {}", target_str, e))
                        })?;
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
                        dup2(src_fd, target_fd)?;
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

impl Drop for RedirectGuard {
    fn drop(&mut self) {
        for (target_fd, saved_fd) in self.saved_fds.drain(..).rev() {
            let _ = dup2(saved_fd, target_fd);
            let _ = nix::unistd::close(saved_fd);
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
}

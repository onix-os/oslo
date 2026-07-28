use crate::ast::{RedirectKind, Redirection};
use crate::env::Environment;
use crate::error::{Result, ShellError};
use crate::expand::expand_word;
use nix::unistd::dup2;
use std::fs::{File, OpenOptions};
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
                    let (reader, writer) = nix::unistd::pipe()?;
                    nix::unistd::write(&writer, content.as_bytes())?;
                    let _ = nix::unistd::close(writer.into_raw_fd());
                    dup2(reader.as_raw_fd(), target_fd)?;
                    let _ = nix::unistd::close(reader.into_raw_fd());
                }
            }
        }
        Ok(())
    }
}

impl Drop for RedirectGuard {
    fn drop(&mut self) {
        for (target_fd, saved_fd) in self.saved_fds.drain(..).rev() {
            let _ = dup2(saved_fd, target_fd);
            let _ = nix::unistd::close(saved_fd);
        }
    }
}

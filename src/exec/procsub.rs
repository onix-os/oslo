//! Process substitution: `<(cmd)` and `>(cmd)`.
//!
//! The command runs in a subshell connected to a pipe, and the *word* becomes the name of that
//! pipe — `/dev/fd/N` — so the program receiving it opens an ordinary file. That is what makes
//! `diff <(sort a) <(sort b)` work without either temporary file existing.
//!
//! Not POSIX, and deferred on that basis until the evidence said otherwise: of 740 real shell
//! scripts on a working system, 74 — one in ten — failed to parse in oslo, and every one of them
//! failed on this. "Not in POSIX" is the wrong test for a shell that intends to be the only one a
//! distribution has.
//!
//! Three things decide the design:
//!
//! * **The descriptor must outlive the expansion.** The word is built during expansion and used
//!   when the command runs, so closing the pipe at the end of expansion would hand the program a
//!   descriptor that is already gone. The shell holds them on `Environment` until the command
//!   that asked for them has finished, and `crate::exec::simple` closes them on every exit path.
//! * **The child must not inherit the descriptor it is named by.** A `<(cmd)` child holding the
//!   read end open means the reader never sees EOF, and `cat <(echo hi)` hangs forever.
//! * **`/dev/fd/N` is a Linux interface**, via `/proc/self/fd`. bash uses the same one here, and
//!   oslo is Linux-only, so there is no fallback path to maintain.

use crate::ast::CommandList;
use crate::env::Environment;
use crate::error::{Result, ShellError};
use crate::exec::compound::flush_stdout;
use crate::exec::pipeline::status_of;
use nix::fcntl::{FcntlArg, FdFlag, fcntl};
use nix::unistd::{ForkResult, Pid, close, dup2, fork, pipe};
use std::os::fd::{IntoRawFd, RawFd};

/// One running substitution: the descriptor the caller was given, and the child feeding it.
pub struct Substitution {
    fd: RawFd,
    child: Pid,
}

/// Start `command` on a pipe and return the `/dev/fd/N` path naming it, plus the handle that
/// keeps it alive.
///
/// A free function rather than a method on the list, because opening one needs `&mut Environment`
/// (the child evaluates in it) and the list lives *on* the environment — the caller records the
/// returned handle once the borrow has ended.
///
/// `reads_from_command` is `<(…)`: the caller reads what the command writes.
pub fn open(
    env: &mut Environment,
    command: &str,
    reads_from_command: bool,
) -> Result<(String, Substitution)> {
    let ast =
        crate::parser::parse_with_aliases(command, &|n| env.get_alias(n).map(str::to_string))?;
    let (reader, writer) =
        pipe().map_err(|e| ShellError::ExecutionError(format!("process substitution: {e}")))?;
    let (ours, theirs) = if reads_from_command {
        (reader.into_raw_fd(), writer.into_raw_fd())
    } else {
        (writer.into_raw_fd(), reader.into_raw_fd())
    };

    // Anything buffered belongs to this shell, not to the child.
    flush_stdout();
    match unsafe { fork() } {
        Ok(ForkResult::Child) => {
            crate::exec::job::reset_signals_for_child();
            // The child must not keep the end the *caller* was named by: while it holds the
            // read end of its own output pipe open, nothing ever sees EOF and the reader hangs.
            let _ = close(ours);
            let target = if reads_from_command { 1 } else { 0 };
            let _ = dup2(theirs, target);
            if theirs != target {
                let _ = close(theirs);
            }
            env.enter_subshell();
            let status = status_of(run(env, &ast));
            flush_stdout();
            std::process::exit(crate::env::builtins::run_exit_trap(env, status));
        }
        Ok(ForkResult::Parent { child }) => {
            let _ = close(theirs);
            // `/dev/fd/N` is opened by the *program*, which is a different process image after
            // `exec` — so the descriptor has to survive it. Rust opens everything `O_CLOEXEC`,
            // and a pipe from `nix` is no exception, so clearing it is what makes the path
            // resolvable on the other side of the `exec`.
            let _ = fcntl(ours, FcntlArg::F_SETFD(FdFlag::empty()));
            Ok((format!("/dev/fd/{ours}"), Substitution { fd: ours, child }))
        }
        Err(e) => {
            let _ = close(ours);
            let _ = close(theirs);
            Err(ShellError::ExecutionError(format!(
                "process substitution: fork failed: {e}"
            )))
        }
    }
}

/// Close every descriptor and reap every child in `open`.
///
/// Closing before reaping is not tidiness: a `>(cmd)` child is *reading* from the pipe, and only
/// the last writer closing gives it EOF. Waiting first would deadlock against a child that is
/// waiting for us.
pub fn finish(open: &mut Vec<Substitution>) {
    for sub in open.iter() {
        let _ = close(sub.fd);
    }
    for sub in open.drain(..) {
        let _ = nix::sys::wait::waitpid(sub.child, None);
    }
}

fn run(env: &mut Environment, ast: &CommandList) -> Result<i32> {
    crate::exec::eval_command_list(env, ast)
}

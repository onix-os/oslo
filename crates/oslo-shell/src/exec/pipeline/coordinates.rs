//! Running a pipeline whose stages read each other by coordinate.
//!
//! ```text
//! cat hosts.txt | ssh {0:0} uptime
//! ```
//!
//! `ssh`'s command line is not known until `cat` has produced a line, so the two cannot run at
//! once. This path runs the stages **one at a time**, keeping what each printed, and rewrites the
//! next stage's words from it before starting it.
//!
//! # Nothing else pays for it
//!
//! [`super::run_stages`] asks [`uses_coordinates`] before any stage runs, and a pipeline that says
//! no goes down the concurrent path it always did. The question is a scan of the literal words for
//! a `{` followed by a digit — no allocation, no capture, nothing kept.
//!
//! # Why the last stage is not captured
//!
//! A captured stage writes to a pipe, and every stage but the last **already** writes to a pipe:
//! standing between them costs nothing, changes no descriptor's nature and is invisible. The last
//! stage writes to the terminal, and interposing there would turn `isatty` false — colours off,
//! pagers not paging, progress bars silent — which is the whole reason `keep` exists as an opt-in
//! rather than a default. So the last stage runs exactly as it always has.
//!
//! That is also why a coordinate reaching *back past this pipeline* is not here yet: it needs the
//! previous command's output, and the previous command was somebody's last stage.
//!
//! # A stage is fed from a file, not a pipe
//!
//! The captured text goes into a temporary file which becomes the next stage's stdin. A pipe would
//! deadlock: the parent cannot write a megabyte into one while the child it is writing to has not
//! started reading, and the child cannot start until the parent stops writing. `tempfile` is a
//! dependency of this crate for the same reason in `direnv`, where the comment says so.

use super::{eval_command, status_of, wait_for_status};
use crate::env::Environment;
use crate::env::builtins::run_exit_trap;
use crate::exec::compound::flush_stdout;
use crate::exec::streams::{self, Streams};
use nix::unistd::{ForkResult, close, dup2, fork, pipe};
use oslo_base::ast::{Command, Pipeline};
use oslo_base::error::{Result, ShellError};
use std::io::{Seek, SeekFrom, Write};
use std::os::fd::{AsRawFd, IntoRawFd};

/// Whether any stage of this pipeline addresses a stream.
///
/// The gate. Answering `false` is the common case and must stay cheap.
pub(super) fn uses_coordinates(pipeline: &Pipeline) -> bool {
    pipeline.commands.iter().any(|command| match command {
        Command::Simple(simple) => streams::command_uses_coordinates(&simple.words),
        // A compound stage — `{ …; }`, a loop, a subshell — is not rewritten. Its words are not a
        // flat list, and a coordinate inside a loop body raises a question this design has not
        // answered: which iteration's stream is it. Left for when somebody wants it.
        _ => false,
    })
}

/// Run the stages one at a time, threading each one's output into the next.
pub(super) fn run(env: &mut Environment, pipeline: &Pipeline) -> Result<i32> {
    let mut streams = Streams::for_this_pipeline();
    let mut statuses = Vec::with_capacity(pipeline.commands.len());
    let mut upstream: Option<String> = None;

    for (at, command) in pipeline.commands.iter().enumerate() {
        let last = at + 1 == pipeline.commands.len();
        let command = rewritten(command, &streams);

        // The last stage is never captured — its stdout is the terminal's, exactly as it always
        // was. See the module docs for why that is not a compromise.
        let (status, kept) = run_stage(env, &command, upstream.as_deref(), !last)?;
        statuses.push(status);
        if last {
            break;
        }
        streams.push_stage(kept.clone());
        upstream = Some(kept);
    }

    let status = *statuses.last().unwrap_or(&0);
    env.set_pipeline_status(statuses);
    Ok(status)
}

/// A copy of the command with its coordinates replaced.
///
/// Cloned rather than rewritten in place: the pipeline belongs to the syntax tree, which a loop may
/// run again, and a stage that rewrote itself would answer the second time around with the first
/// time's text.
fn rewritten(command: &Command, streams: &Streams) -> Command {
    let Command::Simple(simple) = command else {
        return command.clone();
    };
    let mut simple = simple.clone();
    streams::rewrite(&mut simple.words, streams);
    Command::Simple(simple)
}

/// Run one stage: feed it `input`, and keep what it prints when `keep` is set.
///
/// Answers the status and, when kept, the text.
fn run_stage(
    env: &mut Environment,
    command: &Command,
    input: Option<&str>,
    keep: bool,
) -> Result<(i32, String)> {
    let stdin = input.map(as_a_file).transpose()?;
    let capture = keep
        .then(pipe)
        .transpose()
        .map_err(|e| ShellError::ExecutionError(format!("Pipe failed: {e}")))?;
    run_in_child(env, command, stdin.as_ref(), capture)
}

/// The captured text as a file positioned at its start, ready to be a child's stdin.
fn as_a_file(text: &str) -> Result<std::fs::File> {
    let mut file = tempfile::tempfile()
        .map_err(|e| ShellError::ExecutionError(format!("scratch file failed: {e}")))?;
    file.write_all(text.as_bytes())
        .and_then(|()| file.seek(SeekFrom::Start(0)).map(|_| ()))
        .map_err(|e| ShellError::ExecutionError(format!("scratch file failed: {e}")))?;
    Ok(file)
}

/// Fork, rearrange the child's descriptors, run the command in it.
fn run_in_child(
    env: &mut Environment,
    command: &Command,
    stdin: Option<&std::fs::File>,
    capture: Option<(std::os::fd::OwnedFd, std::os::fd::OwnedFd)>,
) -> Result<(i32, String)> {
    // Anything already buffered belongs to the parent, not to what this stage prints.
    flush_stdout();
    let stdin_fd = stdin.map(|file| file.as_raw_fd());

    // Safety: the child only rearranges descriptors it owns and never returns to this function.
    match unsafe { fork() } {
        Ok(ForkResult::Child) => {
            crate::exec::job::reset_signals_for_child();
            if let Some(fd) = stdin_fd {
                let _ = dup2(fd, 0);
            }
            if let Some((reader, writer)) = capture {
                let _ = close(reader.into_raw_fd());
                let _ = dup2(writer.as_raw_fd(), 1);
                let _ = close(writer.into_raw_fd());
            }
            env.enter_subshell();
            let status = status_of(eval_command(env, command));
            // Without this the capture would close on an unflushed partial line.
            flush_stdout();
            std::process::exit(run_exit_trap(env, status));
        }
        Ok(ForkResult::Parent { child }) => {
            let text = match capture {
                Some((reader, writer)) => {
                    let _ = close(writer.into_raw_fd());
                    let mut out = Vec::new();
                    use std::io::Read;
                    let _ = std::fs::File::from(reader).read_to_end(&mut out);
                    // A shell word is a C string and cannot carry a NUL, so a coordinate reading
                    // one would build an argv that dies at exec. Dropped the way command
                    // substitution drops them.
                    out.retain(|&b| b != 0);
                    String::from_utf8_lossy(&out).into_owned()
                }
                None => String::new(),
            };
            Ok((wait_for_status(child), text))
        }
        Err(e) => Err(ShellError::ExecutionError(format!("Fork failed: {e}"))),
    }
}

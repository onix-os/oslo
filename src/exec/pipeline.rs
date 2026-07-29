//! Evaluating command lists, and-or lists and pipelines.
//!
//! The top of the evaluator: a script is a list of and-or lists, each a chain of pipelines,
//! each a chain of commands connected by pipes. Individual commands are handed to
//! [`crate::exec::simple`] or [`crate::exec::compound`].
//!
//! This module also owns the three things every fork site in the shell needs to agree on:
//! [`status_of`] (what an evaluation result becomes when the only channel left is an exit
//! status), [`wait_for_status`] (what a reaped child's wait status becomes), and
//! [`set_interactive`] (whether there is a person watching). Subshells and command
//! substitutions live in other modules but must use these, or the same status bug reappears
//! once per fork site.

use crate::ast::*;
use crate::env::Environment;
use crate::error::{Result, ShellError};
use crate::exec::compound::{eval_compound_command, flush_stdout};
use crate::exec::redirect::RedirectGuard;
use crate::exec::simple::{eval_simple_command, report_redirect_failure};
use nix::fcntl::{OFlag, open};
use nix::sys::stat::Mode;
use nix::sys::wait::{WaitStatus, waitpid};
use nix::unistd::{ForkResult, Pid, close, dup2, fork, pipe};
use std::os::fd::{AsRawFd, IntoRawFd};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

/// Whether this shell is talking to a person.
///
/// Process-global rather than a field on [`Environment`] because it is a property of the
/// invocation, not of a scope, and every forked child inherits it as it stands. `main` sets it
/// when it starts the REPL; a `-c` command or a script leaves it false, which is what keeps the
/// job notice out of `x=$(cmd &)`.
static INTERACTIVE: AtomicBool = AtomicBool::new(false);

/// Job numbers for the `[n] pid` notice, in the order jobs were started.
///
/// A placeholder for the real job table (Round 7): a counter cannot recycle numbers when a job
/// finishes the way bash does, but it does produce bash's *shape*, which is what the notice is
/// judged on until `jobs`/`fg`/`bg` exist.
static NEXT_JOB_NUMBER: AtomicU32 = AtomicU32::new(1);

/// Declare whether the shell is interactive. Called once, by `main`.
pub fn set_interactive(yes: bool) {
    INTERACTIVE.store(yes, Ordering::Relaxed);
}

/// Whether the shell is interactive; see [`set_interactive`].
pub fn is_interactive() -> bool {
    INTERACTIVE.load(Ordering::Relaxed)
}

/// Collapse an evaluation result to the single number a caller with no richer channel can report.
///
/// This is the *only* correct way to turn a `Result<i32>` into an exit status in a forked child
/// — a subshell, a pipeline stage, a background job, a command substitution. Writing
/// `.unwrap_or(1)` there instead is what made `( exit 3 )` report 1: `exit`/`return` unwind as
/// errors carrying their code, and discarding the error discards the code along with the
/// diagnostic for every real failure.
pub fn status_of(result: Result<i32>) -> i32 {
    match result {
        Ok(status) => status,
        Err(err) => report_error_status(err),
    }
}

/// Report `err` and give the status it leaves behind.
///
/// Control flow (`exit`, `return`, `break`, `continue`) passes through silently with its own
/// code. A genuine failure is announced on stderr first: in a forked child this is the last
/// place the diagnostic can be printed at all, since the parent will only ever see a number.
pub fn report_error_status(err: ShellError) -> i32 {
    if let Some(status) = err.control_flow_status() {
        return status;
    }
    eprintln!("rush: {}", err);
    err.failure_status()
}

/// Reap `pid` and turn its wait status into the number a shell reports for it.
///
/// One `waitpid` call, then a `match` on the bound result. Asking twice — once per `if let` arm —
/// cannot work: the second call has no child left to reap, so the `Signaled` arm never fires and
/// a killed child reports as though it had exited cleanly.
pub fn wait_for_status(pid: Pid) -> i32 {
    match waitpid(pid, None) {
        Ok(WaitStatus::Exited(_, code)) => code,
        Ok(WaitStatus::Signaled(_, sig, _)) => 128 + sig as i32,
        // Nothing was reaped, so there is no status to report: 127 is what a shell says when it
        // cannot run or account for a command.
        _ => 127,
    }
}

pub fn eval_command_list(env: &mut Environment, cmd_list: &CommandList) -> Result<i32> {
    let mut last_status = 0;

    for item in &cmd_list.items {
        if item.op == ListOp::Background {
            spawn_background(env, &item.and_or)?;
            // Starting a job always succeeds from the list's point of view; the job's own status
            // is collected later by `wait`.
            last_status = 0;
            env.last_status = 0;
        } else {
            last_status = eval_and_or_list(env, &item.and_or)?;
        }
    }

    Ok(last_status)
}

/// Start `and_or` in the background and record it as `$!`.
fn spawn_background(env: &mut Environment, and_or: &AndOrList) -> Result<()> {
    // R4.1: the parent's buffered output must not be duplicated by the fork.
    flush_stdout();
    unsafe {
        match fork() {
            Ok(ForkResult::Child) => {
                // R4.7: same fresh signal state every forked child gets.
                crate::exec::job::reset_signals_for_child();
                // R4.1: a background child is this shell in another process — it keeps
                // the environment `fork` copied instead of rebuilding one.
                env.enter_subshell();
                detach_stdin();
                let res = status_of(eval_and_or_list(env, and_or));
                flush_stdout();
                std::process::exit(res);
            }
            Ok(ForkResult::Parent { child }) => {
                env.last_bg_pid = Some(child.as_raw() as u32);
                announce_background_job(child);
                Ok(())
            }
            Err(e) => Err(ShellError::ExecutionError(format!("Fork failed: {}", e))),
        }
    }
}

/// Give a background child `/dev/null` for stdin.
///
/// Without job control there is no second process group to keep it off the terminal, so a
/// background `read` competes with the shell for the user's keystrokes and swallows the ones it
/// wins. POSIX asks for exactly this substitution when job control is off. An interactive shell
/// with job control (Round 7) will instead put the job in its own group and leave stdin alone.
///
/// Called between `fork` and any exec, where only async-signal-safe calls are legal: `open`,
/// `dup2` and `close` all are.
fn detach_stdin() {
    if is_interactive() {
        return;
    }
    if let Ok(fd) = open("/dev/null", OFlag::O_RDONLY, Mode::empty()) {
        // Errors are unreportable here and leaving the inherited descriptor is no worse than
        // failing to start the job.
        let _ = dup2(fd, 0);
        if fd != 0 {
            let _ = close(fd);
        }
    }
}

/// Tell the user a job started — but only if there is a user, and never on stdout.
///
/// `println!` here was captured verbatim by every `$( )` containing a `&`, so
/// `x=$(sleep 0 & echo done)` came back as `[bg] 3881847 done`. A non-interactive shell prints
/// no notice at all, which is what bash does and what the corpus compares against.
fn announce_background_job(child: Pid) {
    if !is_interactive() {
        return;
    }
    let job = NEXT_JOB_NUMBER.fetch_add(1, Ordering::Relaxed);
    eprintln!("[{}] {}", job, child);
}

pub fn eval_and_or_list(env: &mut Environment, and_or: &AndOrList) -> Result<i32> {
    let mut status = run_and_record(env, &and_or.first)?;

    for (op, next_pipeline) in &and_or.rest {
        match op {
            AndOrOp::And => {
                if status == 0 {
                    status = run_and_record(env, next_pipeline)?;
                }
            }
            AndOrOp::Or => {
                if status != 0 {
                    status = run_and_record(env, next_pipeline)?;
                }
            }
        }
    }

    Ok(status)
}

/// Run one pipeline and publish its status as `$?` before anything else can read it.
///
/// `$?` used to be written once per top-level list item, so the right-hand side of an and-or list
/// saw the status from *before* the list started: `true; false || echo $?` printed 0. Every
/// pipeline is a command whose status POSIX makes visible to the next one, and the and-or
/// operators are exactly where that is observable.
fn run_and_record(env: &mut Environment, pipeline: &Pipeline) -> Result<i32> {
    let status = eval_pipeline(env, pipeline)?;
    env.last_status = status;
    Ok(status)
}

pub fn eval_pipeline(env: &mut Environment, pipeline: &Pipeline) -> Result<i32> {
    if pipeline.commands.is_empty() {
        return Ok(0);
    }

    if pipeline.commands.len() == 1 {
        let status = eval_command(env, &pipeline.commands[0])?;
        // R4.10: a one-command pipeline still has a stage vector, as bash's `PIPESTATUS` does.
        env.set_pipeline_status(vec![status]);
        return Ok(if pipeline.negated {
            if status == 0 { 1 } else { 0 }
        } else {
            status
        });
    }

    let num_cmds = pipeline.commands.len();
    let mut pipes = Vec::new();

    for _ in 0..num_cmds - 1 {
        let p = pipe()
            .map_err(|e| ShellError::ExecutionError(format!("Pipe creation failed: {}", e)))?;
        pipes.push(p);
    }

    let mut pids = Vec::new();

    for (idx, cmd) in pipeline.commands.iter().enumerate() {
        unsafe {
            match fork() {
                Ok(ForkResult::Child) => {
                    // R4.7: a stage is a new process running commands, so it gets a fresh signal
                    // state — otherwise the Rust runtime's inherited `SIG_IGN` for SIGPIPE turns
                    // `while :; do echo x; done | head -1` from an instant death on the closed
                    // pipe into a loop that spins to completion swallowing EPIPE.
                    crate::exec::job::reset_signals_for_child();
                    if idx > 0 {
                        let prev_read = pipes[idx - 1].0.as_raw_fd();
                        let _ = dup2(prev_read, 0);
                    }
                    if idx < num_cmds - 1 {
                        let curr_write = pipes[idx].1.as_raw_fd();
                        let _ = dup2(curr_write, 1);
                    }

                    for p in &pipes {
                        let _ = close(p.0.as_raw_fd());
                        let _ = close(p.1.as_raw_fd());
                    }

                    // R4.1: a stage is a subshell — same environment, subshell-local state only.
                    env.enter_subshell();
                    // R4.2: `echo x | exit 3` exits the stage with 3, not with 1.
                    let status = status_of(eval_command(env, cmd));
                    flush_stdout();
                    std::process::exit(status);
                }
                Ok(ForkResult::Parent { child }) => {
                    pids.push(child);
                }
                Err(e) => return Err(ShellError::ExecutionError(format!("Fork failed: {}", e))),
            }
        }
    }

    for p in pipes {
        let _ = close(p.0.into_raw_fd());
        let _ = close(p.1.into_raw_fd());
    }

    // R4.10: every stage's status is kept, not just the last one, so `false | true` leaves a
    // trace of the failure for `PIPESTATUS` (Round 8) and `pipefail` (Round 6) to read.
    let mut stage_statuses = Vec::with_capacity(pids.len());
    for pid in pids {
        // R4.3: exactly one `waitpid` per child, inside `wait_for_status`.
        stage_statuses.push(wait_for_status(pid));
    }
    let final_status = *stage_statuses.last().unwrap_or(&0);
    env.set_pipeline_status(stage_statuses);

    Ok(if pipeline.negated {
        if final_status == 0 { 1 } else { 0 }
    } else {
        final_status
    })
}

pub fn eval_command(env: &mut Environment, command: &Command) -> Result<i32> {
    match command {
        Command::Simple(simple) if is_assignment_only(simple) => {
            assignment_only_status(env, simple)
        }
        Command::Simple(simple) => eval_simple_command(env, simple),
        Command::Compound { kind, redirections } => {
            let mut guard = RedirectGuard::new();
            // R4.8: a redirection the shell cannot perform fails the *command*, it does not abort
            // the script — `{ echo hi; } < /nonexistent` prints a diagnostic, leaves `$?` at 1 and
            // the next command still runs. Propagating the error here made it fatal, which is the
            // one thing bash reserves for a special builtin in POSIX mode.
            if let Err(e) = guard.apply(env, redirections) {
                return Ok(report_redirect_failure(&e));
            }
            eval_compound_command(env, kind)
        }
        Command::FunctionDef { name, body } => {
            env.set_function(name, *body.clone());
            Ok(0)
        }
    }
}

/// A command that is nothing but variable assignments: `x=1`, `x=$(cmd) y=2`.
fn is_assignment_only(simple: &SimpleCommand) -> bool {
    simple.words.is_empty() && !simple.assignments.is_empty()
}

/// Run an assignment-only command and give it the status POSIX 2.9.1 assigns it: that of the
/// last command substitution performed, or 0 if there was none.
///
/// `x=$(exit 5)` reports 5 and `x=plain` reports 0 — an assignment cannot itself fail, so the
/// only thing left to report is what ran inside it. Returning a flat `Ok(0)` hid every failure a
/// script captures this way, and `out=$(cmd) || handle` is a common idiom.
///
/// `$?` deliberately still holds the *previous* command's status while the right-hand sides are
/// expanded, so `x=$?` keeps working; the substitution's own status travels on the side channel
/// and only becomes `$?` once the whole command is done.
fn assignment_only_status(env: &mut Environment, simple: &SimpleCommand) -> Result<i32> {
    // Discard anything a previous command left: `echo $(false); x=1` reports 0, because that
    // substitution belonged to the `echo`.
    env.take_substitution_status();
    // A redirection that could not be performed outranks the substitution (R4.8):
    // `x=$(true) < /nonexistent` is a failed command, whatever the substitution did.
    let status = eval_simple_command(env, simple)?;
    if status != 0 {
        return Ok(status);
    }
    Ok(env.take_substitution_status().unwrap_or(0))
}

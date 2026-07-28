//! Evaluating command lists, and-or lists and pipelines.
//!
//! The top of the evaluator: a script is a list of and-or lists, each a chain of pipelines,
//! each a chain of commands connected by pipes. Individual commands are handed to
//! [`crate::exec::simple`] or [`crate::exec::compound`].

use crate::ast::*;
use crate::env::Environment;
use crate::error::{Result, ShellError};
use crate::exec::compound::eval_compound_command;
use crate::exec::redirect::RedirectGuard;
use crate::exec::simple::eval_simple_command;
use nix::sys::wait::{WaitStatus, waitpid};
use nix::unistd::{ForkResult, close, dup2, fork, pipe};
use std::os::fd::{AsRawFd, IntoRawFd};

pub fn eval_command_list(env: &mut Environment, cmd_list: &CommandList) -> Result<i32> {
    let mut last_status = 0;

    for item in &cmd_list.items {
        if item.op == ListOp::Background {
            let mut sub_env = env.get_all_vars();
            let and_or = item.and_or.clone();

            unsafe {
                match fork() {
                    Ok(ForkResult::Child) => {
                        let mut child_env = Environment::new();
                        for (k, v) in sub_env.drain() {
                            child_env.set_var(&k, &v, true);
                        }
                        let res = eval_and_or_list(&mut child_env, &and_or).unwrap_or(1);
                        std::process::exit(res);
                    }
                    Ok(ForkResult::Parent { child }) => {
                        env.last_bg_pid = Some(child.as_raw() as u32);
                        println!("[bg] {}", child);
                        last_status = 0;
                    }
                    Err(e) => {
                        return Err(ShellError::ExecutionError(format!("Fork failed: {}", e)));
                    }
                }
            }
        } else {
            last_status = eval_and_or_list(env, &item.and_or)?;
            env.last_status = last_status;
        }
    }

    Ok(last_status)
}

pub fn eval_and_or_list(env: &mut Environment, and_or: &AndOrList) -> Result<i32> {
    let mut status = eval_pipeline(env, &and_or.first)?;

    for (op, next_pipeline) in &and_or.rest {
        match op {
            AndOrOp::And => {
                if status == 0 {
                    status = eval_pipeline(env, next_pipeline)?;
                }
            }
            AndOrOp::Or => {
                if status != 0 {
                    status = eval_pipeline(env, next_pipeline)?;
                }
            }
        }
    }

    Ok(status)
}

pub fn eval_pipeline(env: &mut Environment, pipeline: &Pipeline) -> Result<i32> {
    if pipeline.commands.is_empty() {
        return Ok(0);
    }

    if pipeline.commands.len() == 1 {
        let status = eval_command(env, &pipeline.commands[0])?;
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

                    let status = eval_command(env, cmd).unwrap_or(1);
                    use std::io::Write;
                    let _ = std::io::stdout().flush();
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

    let mut final_status = 0;
    for (idx, pid) in pids.into_iter().enumerate() {
        if idx == num_cmds - 1 {
            if let Ok(WaitStatus::Exited(_, code)) = waitpid(pid, None) {
                final_status = code;
            } else if let Ok(WaitStatus::Signaled(_, sig, _)) = waitpid(pid, None) {
                final_status = 128 + sig as i32;
            }
        } else {
            let _ = waitpid(pid, None);
        }
    }

    Ok(if pipeline.negated {
        if final_status == 0 { 1 } else { 0 }
    } else {
        final_status
    })
}

pub fn eval_command(env: &mut Environment, command: &Command) -> Result<i32> {
    match command {
        Command::Simple(simple) => eval_simple_command(env, simple),
        Command::Compound { kind, redirections } => {
            let mut guard = RedirectGuard::new();
            guard.apply(env, redirections)?;
            eval_compound_command(env, kind)
        }
        Command::FunctionDef { name, body } => {
            env.set_function(name, *body.clone());
            Ok(0)
        }
    }
}

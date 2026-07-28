//! Evaluating compound commands: `if`, `while`, `until`, `for`, `case`, groups, subshells.
//!
//! `break` and `continue` arrive here as errors unwinding from the loop body; each loop peels
//! one level off the requested depth and either stops or re-raises.

use crate::ast::*;
use crate::env::Environment;
use crate::error::{Result, ShellError};
use crate::exec::pipeline::eval_command_list;
use crate::expand::{expand_word, expand_word_to_string};
use nix::sys::wait::{WaitStatus, waitpid};
use nix::unistd::{ForkResult, fork};

/// What one iteration of a loop body decided.
enum LoopStep {
    /// Carry on with the next iteration.
    Next,
    /// Leave this loop, normally.
    Stop,
    /// Something must escape this loop: an outer `break`/`continue`, a `return`, or an error.
    Unwind(ShellError),
}

/// Run a loop body once and classify the outcome.
///
/// `break n` / `continue n` for `n > 1` are re-raised with the depth decremented, so each
/// enclosing loop peels off one level.
fn run_loop_body(env: &mut Environment, body: &CommandList, status: &mut i32) -> LoopStep {
    match eval_command_list(env, body) {
        Ok(st) => {
            *status = st;
            LoopStep::Next
        }
        Err(ShellError::Break(depth)) if depth > 1 => {
            LoopStep::Unwind(ShellError::Break(depth - 1))
        }
        Err(ShellError::Break(_)) => LoopStep::Stop,
        Err(ShellError::Continue(depth)) if depth > 1 => {
            LoopStep::Unwind(ShellError::Continue(depth - 1))
        }
        Err(ShellError::Continue(_)) => LoopStep::Next,
        Err(e) => LoopStep::Unwind(e),
    }
}

/// Shared driver for `while` and `until`, which differ only in how they read the condition.
///
/// The loop counter is entered and exited around the whole construct rather than around the body
/// so that it stays balanced on every exit path, including an error escaping the condition.
fn eval_conditional_loop(
    env: &mut Environment,
    condition: &CommandList,
    body: &CommandList,
    run_while_zero: bool,
) -> Result<i32> {
    let mut status = 0;
    env.enter_loop();

    let result = loop {
        let cond = match eval_command_list(env, condition) {
            Ok(c) => c,
            Err(e) => break Err(e),
        };
        if (cond == 0) != run_while_zero {
            break Ok(status);
        }

        match run_loop_body(env, body, &mut status) {
            LoopStep::Next => {}
            LoopStep::Stop => break Ok(status),
            LoopStep::Unwind(e) => break Err(e),
        }
    };

    env.exit_loop();
    result
}

pub(crate) fn eval_compound_command(
    env: &mut Environment,
    compound: &CompoundCommand,
) -> Result<i32> {
    match compound {
        CompoundCommand::If {
            condition,
            then_branch,
            elif_branches,
            else_branch,
        } => {
            let cond_status = eval_command_list(env, condition)?;
            if cond_status == 0 {
                return eval_command_list(env, then_branch);
            }

            for (elif_cond, elif_body) in elif_branches {
                let elif_status = eval_command_list(env, elif_cond)?;
                if elif_status == 0 {
                    return eval_command_list(env, elif_body);
                }
            }

            if let Some(else_b) = else_branch {
                eval_command_list(env, else_b)
            } else {
                Ok(0)
            }
        }
        CompoundCommand::While { condition, body } => {
            eval_conditional_loop(env, condition, body, true)
        }
        CompoundCommand::Until { condition, body } => {
            eval_conditional_loop(env, condition, body, false)
        }
        CompoundCommand::For {
            var_name,
            items,
            body,
        } => {
            let item_words = if let Some(it) = items {
                let mut vec = Vec::new();
                for w in it {
                    vec.extend(expand_word(env, w)?);
                }
                vec
            } else {
                env.get_positional().to_vec()
            };

            let mut status = 0;
            env.enter_loop();
            let result = 'items: {
                for item in item_words {
                    env.set_var(var_name, &item, false);
                    match run_loop_body(env, body, &mut status) {
                        LoopStep::Next => {}
                        LoopStep::Stop => break 'items Ok(status),
                        LoopStep::Unwind(e) => break 'items Err(e),
                    }
                }
                Ok(status)
            };
            env.exit_loop();
            result
        }
        CompoundCommand::Case { word, items } => {
            // Neither the subject nor the patterns are field-split or pathname-expanded here;
            // `expand_word` would glob `f*` against the working directory instead of leaving it
            // as a pattern to match against.
            let expanded = expand_word_to_string(env, word)?;

            for item in items {
                let mut matched = false;
                for pat_word in &item.patterns {
                    let pat_str = expand_word_to_string(env, pat_word)?;
                    if glob::Pattern::new(&pat_str)
                        .map(|p| p.matches(&expanded))
                        .unwrap_or(pat_str == expanded)
                    {
                        matched = true;
                        break;
                    }
                }

                if matched {
                    return eval_command_list(env, &item.body);
                }
            }

            Ok(0)
        }
        CompoundCommand::Subshell(body) => {
            let vars = env.get_all_vars();
            unsafe {
                match fork() {
                    Ok(ForkResult::Child) => {
                        let mut child_env = Environment::new();
                        for (k, v) in vars {
                            child_env.set_var(&k, &v, true);
                        }
                        let res = eval_command_list(&mut child_env, body).unwrap_or(1);
                        std::process::exit(res);
                    }
                    Ok(ForkResult::Parent { child }) => {
                        if let Ok(WaitStatus::Exited(_, code)) = waitpid(child, None) {
                            Ok(code)
                        } else {
                            Ok(1)
                        }
                    }
                    Err(e) => Err(ShellError::ExecutionError(format!(
                        "Subshell fork failed: {}",
                        e
                    ))),
                }
            }
        }
        CompoundCommand::Group(body) => eval_command_list(env, body),
    }
}

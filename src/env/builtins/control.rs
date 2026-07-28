//! Control flow and script loading: `break`, `continue`, `return`, `exit`, `type`, `eval`,
//! `source`.

use crate::env::scope::Environment;
use crate::error::{Result, ShellError};
use crate::exec::eval_command_list;
use std::fs;

fn loop_depth(name: &str, args: &[String]) -> std::result::Result<usize, i32> {
    match args.get(1) {
        None => Ok(1),
        Some(raw) => match raw.parse::<usize>() {
            Ok(0) | Err(_) => {
                eprintln!("rush: {}: {}: numeric argument required", name, raw);
                Err(1)
            }
            Ok(n) => Ok(n),
        },
    }
}

/// `break [n]` — leave the innermost `n` enclosing loops.
///
/// Signalled as an error so it unwinds through nested command lists; the loop evaluators in
/// [`crate::exec::pipeline`] catch it, decrement the depth, and either stop or re-raise.
pub fn builtin_break(env: &mut Environment, args: &[String]) -> Result<i32> {
    match loop_depth("break", args) {
        // Outside a loop this is a silent no-op: signalling would unwind out of the enclosing
        // command list and abandon the commands after it.
        Ok(_) if !env.in_loop() => Ok(0),
        Ok(n) => Err(ShellError::Break(n)),
        Err(code) => Ok(code),
    }
}

/// `continue [n]` — start the next iteration of the `n`th enclosing loop.
pub fn builtin_continue(env: &mut Environment, args: &[String]) -> Result<i32> {
    match loop_depth("continue", args) {
        Ok(_) if !env.in_loop() => Ok(0),
        Ok(n) => Err(ShellError::Continue(n)),
        Err(code) => Ok(code),
    }
}

/// `return [n]` — return from a function or sourced script with status `n`.
pub fn builtin_return(env: &mut Environment, args: &[String]) -> Result<i32> {
    let code = match args.get(1) {
        // Bare `return` yields the status of the last command, as in bash.
        None => env.last_status,
        Some(raw) => match raw.parse::<i32>() {
            Ok(n) => n,
            Err(_) => {
                eprintln!("rush: return: {}: numeric argument required", raw);
                return Ok(1);
            }
        },
    };
    Err(ShellError::Return(code))
}

pub fn builtin_exit(_env: &mut Environment, args: &[String]) -> Result<i32> {
    let code = if args.len() > 1 {
        args[1].parse::<i32>().unwrap_or(0)
    } else {
        0
    };
    Err(ShellError::Exit(code))
}

pub fn builtin_type(env: &mut Environment, args: &[String]) -> Result<i32> {
    if args.len() < 2 {
        return Ok(0);
    }

    let mut status = 0;
    for arg in &args[1..] {
        if env.is_builtin(arg) {
            println!("{} is a shell builtin", arg);
        } else if let Some(alias) = env.get_alias(arg) {
            println!("{} is aliased to `{}`", arg, alias);
        } else if env.get_function(arg).is_some() {
            println!("{} is a function", arg);
        } else if let Ok(path) = which::which(arg) {
            println!("{} is {}", arg, path.display());
        } else {
            eprintln!("rush: type: {}: not found", arg);
            status = 1;
        }
    }

    Ok(status)
}

pub fn builtin_eval(env: &mut Environment, args: &[String]) -> Result<i32> {
    if args.len() < 2 {
        return Ok(0);
    }

    let code = args[1..].join(" ");

    // `x='eval "$x"'; eval "$x"` recurses through the parser and the evaluator with no function
    // call to bound it, so `eval` carries the same nesting counter `source` does.
    env.enter_nested_script()?;
    let result = match crate::parser::parse_bash_script(&code) {
        Ok(ast) => eval_command_list(env, &ast),
        // A syntax error in evaluated text is the *builtin's* failure, not the script's: bash
        // reports it, gives `eval` status 2, and carries on with the next command.
        Err(e) => {
            eprintln!("rush: eval: {}", e);
            Ok(2)
        }
    };
    env.exit_nested_script();
    result
}

pub fn builtin_source(env: &mut Environment, args: &[String]) -> Result<i32> {
    if args.len() < 2 {
        eprintln!("rush: source: filename argument required");
        return Ok(1);
    }

    let file_path = &args[1];
    let content = match fs::read_to_string(file_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("rush: source: {}: {}", file_path, e);
            return Ok(1);
        }
    };

    // A file that sources itself re-enters the parser and the evaluator once per level. The
    // counter is entered only after the file is known to be readable, so a missing file still
    // costs nothing, and exited on every path out so a `return` cannot leave it drifting.
    env.enter_nested_script()?;
    let result = match crate::parser::parse_bash_script(&content) {
        Ok(ast) => eval_command_list(env, &ast),
        // As with `eval`: the sourced file failing to parse leaves `source` with status 2 and
        // the calling script still running.
        Err(e) => {
            eprintln!("rush: {}: {}", file_path, e);
            Ok(2)
        }
    };
    env.exit_nested_script();

    // `return` ends a sourced script early and supplies its status.
    match result {
        Err(ShellError::Return(code)) => Ok(code),
        other => other,
    }
}

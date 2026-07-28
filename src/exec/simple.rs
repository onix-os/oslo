//! Evaluating a simple command.
//!
//! Expansion, alias substitution, command-prefix assignments, then dispatch to a builtin, a
//! shell function, or an external binary.

use crate::ast::*;
use crate::env::Environment;
use crate::error::{Result, ShellError};
use crate::exec::pipeline::eval_command;
use crate::exec::redirect::RedirectGuard;
use crate::expand::expand_word;
use crate::lexer::Lexer;
use nix::sys::wait::{WaitStatus, waitpid};
use nix::unistd::{ForkResult, fork};
use std::ffi::CString;

pub(crate) fn eval_simple_command(env: &mut Environment, simple: &SimpleCommand) -> Result<i32> {
    if simple.words.is_empty() {
        for assign in &simple.assignments {
            let expanded = expand_word(env, &assign.value)?;
            let val_str = expanded.join(" ");
            env.set_var(&assign.name, &val_str, false);
        }
        return Ok(0);
    }

    let mut words = Vec::new();
    for w in &simple.words {
        words.extend(expand_word(env, w)?);
    }

    if words.is_empty() {
        for assign in &simple.assignments {
            let expanded = expand_word(env, &assign.value)?;
            let val_str = expanded.join(" ");
            env.set_var(&assign.name, &val_str, false);
        }
        return Ok(0);
    }

    let raw_name = words[0].trim().to_string();

    // Alias expansion replaces the command word with the alias body, which may itself be several
    // words: `alias ll='ls -la'` has to become argv `["ls", "-la"]`, not the single argv[0]
    // `"ls -la"`. Expanded once, not recursively, so a self-referential alias terminates.
    if let Some(alias) = env.get_alias(&raw_name).map(|s| s.to_string()) {
        let expanded = expand_alias(env, &alias)?;
        if !expanded.is_empty() {
            let mut rebuilt = expanded;
            rebuilt.extend_from_slice(&words[1..]);
            words = rebuilt;
        }
    }

    let cmd_name = words[0].trim().to_string();
    words[0] = cmd_name.clone();

    let is_declaration = matches!(
        cmd_name.as_str(),
        "export" | "local" | "readonly" | "declare"
    );

    // A prefix assignment on a *declaration* builtin is really that builtin's argument:
    // `export FOO=bar` must reach `export`, not be applied behind its back.
    let mut prefix_assignments = Vec::new();
    for assign in &simple.assignments {
        let val_str = expand_word(env, &assign.value)?.join(" ");
        if is_declaration {
            words.push(format!("{}={}", assign.name, val_str));
        } else {
            prefix_assignments.push((assign.name.clone(), val_str));
        }
    }

    // `FOO=bar cmd` exports FOO for the duration of `cmd` only.
    //
    // The scope is pushed only when there is something to put in it. Pushing unconditionally
    // would give `local` a throwaway frame to write into, so `local V=x` would be undone the
    // moment the command finished.
    if prefix_assignments.is_empty() {
        return run_command_word(env, &cmd_name, &words, &simple.redirections);
    }

    env.push_scope();
    for (name, value) in &prefix_assignments {
        env.set_local_exported_var(name, value);
    }
    let result = run_command_word(env, &cmd_name, &words, &simple.redirections);
    env.pop_scope();
    result
}

/// Dispatch an already-expanded command: builtin, then function, then external binary.
fn run_command_word(
    env: &mut Environment,
    cmd_name: &str,
    words: &[String],
    redirections: &[Redirection],
) -> Result<i32> {
    let cmd_name = cmd_name.to_string();
    let clean_cmd_name = cmd_name.trim();
    if env.is_builtin(clean_cmd_name) {
        let mut guard = RedirectGuard::new();
        guard.apply(env, redirections)?;

        return execute_builtin(env, clean_cmd_name, words);
    }

    if let Some(func_body) = env.get_function(&cmd_name).cloned() {
        let mut guard = RedirectGuard::new();
        guard.apply(env, redirections)?;

        let old_pos = env.get_positional().to_vec();
        env.set_positional(words[1..].to_vec());
        env.push_scope();
        let res = eval_command(env, &func_body);
        env.pop_scope();
        env.set_positional(old_pos);

        // `return` unwinds to here and becomes the function's exit status. `break`/`continue`
        // are also absorbed: they must not escape into a loop in the caller.
        return match res {
            Err(ShellError::Return(code)) => Ok(code),
            Err(ShellError::Break(_)) | Err(ShellError::Continue(_)) => Ok(0),
            other => other,
        };
    }

    let path = match which::which(&cmd_name) {
        Ok(p) => p,
        Err(_) => {
            if words.len() == 1 && std::path::Path::new(&cmd_name).is_dir() {
                return crate::env::builtins::builtin_cd(env, &["cd".to_string(), cmd_name]);
            }
            eprintln!("rush: {}: command not found", cmd_name);
            return Ok(127);
        }
    };

    let c_path = CString::new(path.to_str().unwrap()).unwrap();
    let c_args: Vec<CString> = words
        .iter()
        .map(|w| CString::new(w.as_str()).unwrap())
        .collect();

    unsafe {
        match fork() {
            Ok(ForkResult::Child) => {
                let mut guard = RedirectGuard::new();
                if let Err(e) = guard.apply(env, redirections) {
                    eprintln!("rush: redirection error: {}", e);
                    std::process::exit(1);
                }

                let _ = nix::unistd::execv(&c_path, &c_args);
                eprintln!("rush: exec failed for {}", cmd_name);
                std::process::exit(126);
            }
            Ok(ForkResult::Parent { child }) => match waitpid(child, None) {
                Ok(WaitStatus::Exited(_, code)) => Ok(code),
                Ok(WaitStatus::Signaled(_, sig, _)) => Ok(128 + sig as i32),
                _ => Ok(1),
            },
            Err(e) => Err(ShellError::ExecutionError(format!("Fork failed: {}", e))),
        }
    }
}

/// Split an alias body into argv entries.
///
/// Lexed and expanded the same way as any other command words, so a quoted alias body such as
/// `alias g='grep --color "a b"'` keeps its argument grouping instead of being split on spaces.
fn expand_alias(env: &mut Environment, alias: &str) -> Result<Vec<String>> {
    let mut lexer = Lexer::new(alias);
    let mut out = Vec::new();

    loop {
        match lexer.next() {
            Ok(crate::lexer::Token::Word(w)) => out.extend(expand_word(env, &w)?),
            Ok(crate::lexer::Token::Eof) => break,
            // The alias body contains operators (`alias x='a | b'`), which cannot be represented
            // as a flat argv. Fall back to whitespace splitting rather than dropping them.
            Ok(_) | Err(_) => {
                return Ok(alias.split_whitespace().map(str::to_string).collect());
            }
        }
    }

    Ok(out)
}

fn execute_builtin(env: &mut Environment, cmd_name: &str, words: &[String]) -> Result<i32> {
    match cmd_name {
        "cd" => crate::env::builtins::builtin_cd(env, words),
        "echo" => crate::env::builtins::builtin_echo(env, words),
        "pwd" => crate::env::builtins::builtin_pwd(env, words),
        "export" => crate::env::builtins::builtin_export(env, words),
        "unset" => crate::env::builtins::builtin_unset(env, words),
        "set" => crate::env::builtins::builtin_set(env, words),
        "shift" => crate::env::builtins::builtin_shift(env, words),
        "exit" => crate::env::builtins::builtin_exit(env, words),
        "break" => crate::env::builtins::builtin_break(env, words),
        "continue" => crate::env::builtins::builtin_continue(env, words),
        "return" => crate::env::builtins::builtin_return(env, words),
        "alias" => crate::env::builtins::builtin_alias(env, words),
        "unalias" => crate::env::builtins::builtin_unalias(env, words),
        "type" => crate::env::builtins::builtin_type(env, words),
        "eval" => crate::env::builtins::builtin_eval(env, words),
        "source" | "." => crate::env::builtins::builtin_source(env, words),
        "read" => crate::env::builtins::builtin_read(env, words),
        "local" => crate::env::builtins::builtin_local(env, words),
        "pushd" => crate::env::builtins::builtin_pushd(env, words),
        "popd" => crate::env::builtins::builtin_popd(env, words),
        "dirs" => crate::env::builtins::builtin_dirs(env, words),
        "readonly" => crate::env::builtins::builtin_readonly(env, words),
        "test" | "[" => crate::env::builtins::builtin_test(env, words),
        "[[" => crate::env::builtins::builtin_extended_test(env, words),

        "trap" => crate::env::builtins::builtin_trap(env, words),
        "umask" => crate::env::builtins::builtin_umask(env, words),
        "wait" => crate::env::builtins::builtin_wait(env, words),
        "kill" => crate::env::builtins::builtin_kill(env, words),
        "true" => Ok(0),
        "false" => Ok(1),
        _ => {
            if let Some(res) = env.exec_custom_builtin(cmd_name, words) {
                res
            } else {
                Ok(0)
            }
        }
    }
}

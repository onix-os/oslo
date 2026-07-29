//! Evaluating a simple command.
//!
//! Expansion, alias substitution, command-prefix assignments, then dispatch to a builtin, a
//! shell function, or an external binary.

use crate::ast::*;
use crate::env::Environment;
use crate::error::{Result, ShellError};
use crate::exec::pipeline::eval_command;
use crate::exec::redirect::RedirectGuard;
use crate::expand::{expand_word, expand_word_to_string};
use crate::lexer::Lexer;
use nix::sys::wait::{WaitPidFlag, WaitStatus, waitpid};
use nix::unistd::{ForkResult, fork};
use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;

pub(crate) fn eval_simple_command(env: &mut Environment, simple: &SimpleCommand) -> Result<i32> {
    if simple.words.is_empty() {
        for assign in &simple.assignments {
            // Assignment RHS is *not* field-split or globbed (POSIX 2.9.1), so `x=*.rs` stores the
            // literal pattern and `x=$(printf 'a\nb')` keeps its newline.
            let val_str = expand_word_to_string(env, &assign.value)?;
            env.set_var(&assign.name, &val_str, false);
        }
        return Ok(apply_wordless_redirections(env, &simple.redirections));
    }

    let mut words = Vec::new();
    for w in &simple.words {
        words.extend(expand_word(env, w)?);
    }

    if words.is_empty() {
        for assign in &simple.assignments {
            let val_str = expand_word_to_string(env, &assign.value)?;
            env.set_var(&assign.name, &val_str, false);
        }
        return Ok(apply_wordless_redirections(env, &simple.redirections));
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
        let val_str = expand_word_to_string(env, &assign.value)?;
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
        if let Err(e) = guard.apply(env, redirections) {
            return Ok(report_redirect_failure(&e));
        }

        return execute_builtin(env, clean_cmd_name, words);
    }

    if let Some(func_body) = env.get_function(&cmd_name).cloned() {
        // Checked before anything is set up, so a refused call has nothing to unwind. `f() { f; }`
        // recurses through the whole evaluator; without this the stack overflows and Rust aborts
        // the process outright, status 134 and a core dump.
        env.enter_function()?;
        let res = call_function(env, &func_body, words, redirections);
        env.exit_function();

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

    // Both conversions take the raw bytes: a resolved path is an `OsStr`, not necessarily UTF-8
    // (a PATH entry can be any byte string), and `to_str().unwrap()` aborted the shell on one.
    let c_path = exec_cstring(path.as_os_str().as_bytes());
    let c_args: Vec<CString> = words.iter().map(|w| exec_cstring(w.as_bytes())).collect();

    unsafe {
        match fork() {
            Ok(ForkResult::Child) => {
                // Before anything else, and in particular before `execv`: the program about to
                // replace this process must not inherit the shell's own signal policy.
                crate::exec::job::reset_signals_for_child();

                let mut guard = RedirectGuard::new();
                if let Err(e) = guard.apply(env, redirections) {
                    std::process::exit(report_redirect_failure(&e));
                }

                let _ = nix::unistd::execv(&c_path, &c_args);
                eprintln!("rush: exec failed for {}", cmd_name);
                std::process::exit(126);
            }
            Ok(ForkResult::Parent { child }) => Ok(wait_for_child(child, &cmd_name)),
            Err(e) => Err(ShellError::ExecutionError(format!("Fork failed: {}", e))),
        }
    }
}

/// Run a shell function body with its own positional parameters and variable scope.
///
/// Split out from [`run_command_word`] so the call-depth counter can be entered and exited around
/// exactly one expression: a redirection failure here still has to leave the depth balanced.
fn call_function(
    env: &mut Environment,
    body: &Command,
    words: &[String],
    redirections: &[Redirection],
) -> Result<i32> {
    let mut guard = RedirectGuard::new();
    if let Err(e) = guard.apply(env, redirections) {
        // The body does not run at all: `f < /nonexistent` is a failed command, not a call whose
        // stdin happens to be the shell's.
        return Ok(report_redirect_failure(&e));
    }

    let old_pos = env.get_positional().to_vec();
    env.set_positional(words[1..].to_vec());
    env.push_scope();
    let res = eval_command(env, body);
    env.pop_scope();
    env.set_positional(old_pos);
    res
}

/// Wait for a foreground child and turn its wait status into an exit status.
///
/// `WUNTRACED` is not optional now that children are started with SIGTSTP at `SIG_DFL`
/// ([`crate::exec::job::reset_signals_for_child`]). Ctrl-Z stops the child, and a plain
/// `waitpid` — which only reports *termination* — would then block the shell forever on a
/// process nobody can resume, turning a suspend into a hang. Reporting the stop instead returns
/// the prompt; the process stays stopped until job control (`fg`/`bg`) can adopt it.
///
/// `EINTR` is retried rather than reported: a trapped signal arriving mid-wait says nothing
/// about how the command ended.
///
/// A non-interactive bash *does* block on a stopped child — `bash -c 'sh -c "kill -STOP $$"'`
/// never returns — so this is a deliberate divergence from the oracle, in the one direction
/// where the oracle's behaviour is indistinguishable from a deadlock.
fn wait_for_child(child: nix::unistd::Pid, cmd_name: &str) -> i32 {
    loop {
        match waitpid(child, Some(WaitPidFlag::WUNTRACED)) {
            Ok(WaitStatus::Exited(_, code)) => return code,
            // A shell reports a signal death as 128 + the signal number, which is how `$?` tells
            // `kill -9` (137) apart from an exit status of 9.
            Ok(WaitStatus::Signaled(_, sig, _)) => return 128 + sig as i32,
            Ok(WaitStatus::Stopped(_, sig)) => {
                eprintln!("rush: {}: stopped ({})", cmd_name, sig);
                return 128 + sig as i32;
            }
            Ok(_) => continue,
            Err(nix::errno::Errno::EINTR) => continue,
            Err(_) => return 1,
        }
    }
}

/// Report a redirection failure and hand back the status the failed command takes on.
///
/// A redirection that cannot be set up fails *the command*, not the shell. rush used to propagate
/// the error to `main`, which exited — so `echo hi < /nonexistent; echo CONTINUE` never printed
/// CONTINUE, while the same redirection on an external command continued happily. The two paths
/// disagreed with each other; this is the one place that decides.
///
/// Status 1, measured against `bash --posix` for a builtin (`read x < /nonexistent`), a bad
/// descriptor (`echo hi >&7`), a function, a compound and an external command: all print a
/// diagnostic, set `$?` to 1 and carry on.
///
/// The one case bash treats differently is a redirection error on a *special* builtin (`:`,
/// `export`, …) in POSIX mode, which does abort the shell. rush does not implement the special
/// builtin distinction anywhere yet — see the `robust_special_builtin_failure.sh` corpus case —
/// so it is not invented here; continuing is the behaviour of every non-POSIX-mode shell and of
/// bash for every other command.
pub(crate) fn report_redirect_failure(err: &ShellError) -> i32 {
    eprintln!("rush: {}", err);
    1
}

/// Apply the redirections of a command that has no command word.
///
/// `> out` on its own still creates `out`, and `x=1 < /nonexistent` still fails with status 1
/// after performing the assignment. The guard is dropped immediately, which restores the saved
/// descriptors — the redirection's only lasting effect is on the filesystem.
fn apply_wordless_redirections(env: &mut Environment, redirections: &[Redirection]) -> i32 {
    if redirections.is_empty() {
        return 0;
    }
    let mut guard = RedirectGuard::new();
    match guard.apply(env, redirections) {
        Ok(()) => 0,
        Err(e) => report_redirect_failure(&e),
    }
}

/// Turn user-controlled bytes into an argv entry for `execv`, dropping any NUL bytes.
///
/// argv entries are NUL-terminated, so an embedded NUL cannot reach `execv` under any encoding —
/// the only question is what the shell does about it. bash keeps NULs out of shell strings in the
/// first place (command substitution and `read` both drop them), so dropping here reproduces the
/// argument bash would have built. What it must never do is what the previous
/// `CString::new(..).unwrap()` did: kill the whole shell, exit 101, over one byte of input data.
fn exec_cstring(bytes: &[u8]) -> CString {
    let stripped: Vec<u8> = bytes.iter().copied().filter(|b| *b != 0).collect();
    // NUL-free by construction, so this cannot fail; the fallback keeps the exec path free of
    // panicking calls even if that ever stops being true.
    CString::new(stripped).unwrap_or_default()
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

#[cfg(test)]
mod tests {
    use super::exec_cstring;
    use crate::env::Environment;
    use std::ffi::{CString, OsStr};
    use std::os::unix::ffi::OsStrExt;

    /// Run a snippet in a fresh environment and hand back the environment to inspect.
    ///
    /// `Environment::new()` snapshots the *process* environment and `export` writes back into it,
    /// so an exported name set by one test is visible to every environment built afterwards.
    /// Tests here therefore use names unique to each test rather than a shared `v`.
    fn run(src: &str) -> Environment {
        let mut env = Environment::new();
        let script = crate::parser::parse_bash_script(src).expect("parse");
        crate::exec::eval_command_list(&mut env, &script).expect("exec");
        env
    }

    fn var(src: &str, name: &str) -> String {
        run(src).get_var(name).unwrap_or_default().to_string()
    }

    /// POSIX 2.9.1: the assignment RHS gets tilde, parameter, command and arithmetic expansion,
    /// but *not* field splitting and *not* pathname expansion. Globbing it would make the value
    /// depend on the working directory's contents; splitting it would collapse any IFS character
    /// or newline the value legitimately contains.
    ///
    /// `Cargo.*` is used rather than a scratch directory on purpose: unit tests run in the crate
    /// root, where that pattern really does match files, so a regression to `expand_word` would
    /// show up as `Cargo.lock Cargo.toml` instead of a silently-unchanged literal.
    #[test]
    fn assignment_rhs_is_not_globbed() {
        assert_eq!(var("rush_g1=Cargo.*", "rush_g1"), "Cargo.*");
        assert_eq!(var("rush_g2=Cargo.* true", "rush_g2"), "");
        assert_eq!(var("export rush_g3=Cargo.*", "rush_g3"), "Cargo.*");
    }

    #[test]
    fn assignment_rhs_is_not_field_split() {
        assert_eq!(var("IFS=:\nrush_s1=a:b:c", "rush_s1"), "a:b:c");
        assert_eq!(var("IFS=:\nexport rush_s2=a:b:c", "rush_s2"), "a:b:c");
        // Interior whitespace from an unquoted expansion survives too.
        assert_eq!(var("rush_s3='a  b'\nrush_s4=$rush_s3", "rush_s4"), "a  b");
    }

    // The third leg of R2.9 — `x=$(printf 'a\nb')` keeps its newline — is deliberately *not*
    // tested here. Command substitution forks (`exec/substitution.rs`), and libtest runs unit
    // tests on a pool of threads: a child forked out of a multi-threaded process inherits any
    // mutex another thread happened to hold, so the child deadlocks in the allocator before it
    // can write to the pipe and the parent blocks forever in `waitpid`. That is a property of
    // the harness, not of the shell (rush itself is single-threaded), so the case lives in
    // `tests/expansion_tests.rs`, which spawns the real binary.

    /// The `words.is_empty()` fallback path — a command word that expands to nothing leaves only
    /// the assignments — must apply the same rule as the ordinary one.
    #[test]
    fn assignment_survives_an_empty_command_word() {
        assert_eq!(
            var("IFS=:\nrush_e1=\nrush_e2=a:b $rush_e1", "rush_e2"),
            "a:b"
        );
    }

    /// A prefix assignment is scoped to its command and must not leak back out.
    #[test]
    fn prefix_assignment_does_not_outlive_its_command() {
        assert_eq!(run("rush_p1=a:b true").get_var("rush_p1"), None);
    }

    #[test]
    fn plain_argument_is_unchanged() {
        assert_eq!(exec_cstring(b"hello"), CString::new("hello").unwrap());
        assert_eq!(exec_cstring(b""), CString::new("").unwrap());
    }

    #[test]
    fn embedded_nul_is_dropped_not_fatal() {
        assert_eq!(exec_cstring(b"a\0b"), CString::new("ab").unwrap());
        assert_eq!(exec_cstring(b"\0\0"), CString::new("").unwrap());
        assert_eq!(exec_cstring(b"a\0"), CString::new("a").unwrap());
        assert_eq!(exec_cstring(b"\0a"), CString::new("a").unwrap());
    }

    /// A PATH entry — and therefore a resolved binary path — need not be UTF-8.
    #[test]
    fn non_utf8_path_survives() {
        let raw = b"/b\xffn/echo";
        assert_eq!(
            exec_cstring(OsStr::from_bytes(raw).as_bytes()).as_bytes(),
            raw
        );
    }
}

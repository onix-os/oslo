//! Control flow and script loading: `break`, `continue`, `return`, `exit`, `type`, `eval`,
//! `source`.
//!
//! `type` lives in `resolve` because reporting how a name resolves is a different job from
//! running it, and the source printer it needs to show a function body lives in `unparse`.

mod resolve;
mod unparse;

pub use resolve::builtin_type;
/// Whether a name is a reserved word; `command -v` has to agree with `type` about this.
pub use resolve::is_keyword;
/// Render a function definition as shell source; also what `set` and `declare -f` need to print.
pub use unparse::format_function;

use crate::env::scope::Environment;
use crate::error::{Result, ShellError};
use crate::exec::eval_command_list;
use std::fs;

/// Parse a builtin's numeric operand the way bash's `legal_number` does.
///
/// Surrounding blanks are allowed (`exit " 5 "` is 5 in bash) but nothing else is: a name that
/// merely *starts* with digits is not a number, or `exit 1x` would silently exit 1.
fn numeric_operand<T: std::str::FromStr>(name: &str, raw: &str) -> std::result::Result<T, ()> {
    raw.trim().parse::<T>().map_err(|_| {
        eprintln!("oslo: {}: {}: numeric argument required", name, raw);
    })
}

fn loop_depth(name: &str, args: &[String]) -> std::result::Result<usize, i32> {
    match args.get(1) {
        None => Ok(1),
        Some(raw) => match numeric_operand::<usize>(name, raw) {
            Err(()) => Err(1),
            // `break 0` is not a loop count; the message is the same one bash gives a
            // non-numeric operand, and `numeric_operand` has not printed it in this branch.
            Ok(0) => {
                eprintln!("oslo: {}: {}: numeric argument required", name, raw);
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
        // A bad operand still returns — bash unwinds the function with status 2 rather than
        // carrying on with the commands after the `return`.
        Some(raw) => numeric_operand::<i32>("return", raw).unwrap_or(2),
    };
    Err(ShellError::Return(exit_status(code)))
}

/// `exit [n]` — end the shell with status `n`.
///
/// A garbage operand used to `unwrap_or(0)`, so `exit abc` reported *success* — the single worst
/// answer available, since a script that miscomputed its status is exactly what an exit status is
/// meant to reveal. It is a usage error, and bash leaves with 2.
pub fn builtin_exit(env: &mut Environment, args: &[String]) -> Result<i32> {
    if args.len() > 2 {
        eprintln!("oslo: exit: too many arguments");
        // Not an exit: bash refuses the request and leaves the shell running.
        return Ok(1);
    }
    let code = match args.get(1) {
        // Bare `exit` carries the last command's status out, so `cmd; exit` is `cmd; exit $?`.
        None => env.last_status,
        Some(raw) => match numeric_operand::<i64>("exit", raw) {
            Ok(n) => n as i32,
            Err(()) => return Err(ShellError::Exit(2)),
        },
    };
    Err(ShellError::Exit(exit_status(code)))
}

/// Fold a requested status into the 0..=255 a process can actually report.
///
/// `exit 300` is 44 and `exit -1` is 255 in every shell, because only the low byte survives
/// `wait`. Folding here rather than at `process::exit` keeps `$?` inside a subshell agreeing
/// with `$?` outside one.
fn exit_status(code: i32) -> i32 {
    code & 0xff
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
            eprintln!("oslo: eval: {}", e);
            Ok(2)
        }
    };
    env.exit_nested_script();
    result
}

pub fn builtin_source(env: &mut Environment, args: &[String]) -> Result<i32> {
    if args.len() < 2 {
        eprintln!("oslo: source: filename argument required");
        return Ok(1);
    }

    let file_path = &args[1];
    let content = match fs::read_to_string(file_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("oslo: source: {}: {}", file_path, e);
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
            eprintln!("oslo: {}: {}", file_path, e);
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

#[cfg(test)]
mod tests {
    use super::{builtin_break, builtin_continue, builtin_exit, builtin_return};
    use crate::env::scope::Environment;
    use crate::error::ShellError;

    fn argv(words: &[&str]) -> Vec<String> {
        words.iter().map(|s| s.to_string()).collect()
    }

    /// `exit abc` used to parse as `unwrap_or(0)` and report **success** — the one answer a
    /// status-reporting builtin must never give. All four control builtins refuse it.
    #[test]
    fn a_non_numeric_operand_is_refused_by_every_control_builtin() {
        let mut env = Environment::new();
        assert!(matches!(
            builtin_exit(&mut env, &argv(&["exit", "abc"])),
            Err(ShellError::Exit(2))
        ));
        assert!(matches!(
            builtin_return(&mut env, &argv(&["return", "abc"])),
            Err(ShellError::Return(2))
        ));
        assert_eq!(
            builtin_break(&mut env, &argv(&["break", "abc"])).expect("no error"),
            1
        );
        assert_eq!(
            builtin_continue(&mut env, &argv(&["continue", "abc"])).expect("no error"),
            1
        );
    }

    /// A digit prefix is not a number: `exit 1x` must not quietly exit 1.
    #[test]
    fn a_partly_numeric_operand_is_not_a_number() {
        let mut env = Environment::new();
        assert!(matches!(
            builtin_exit(&mut env, &argv(&["exit", "1x"])),
            Err(ShellError::Exit(2))
        ));
    }

    #[test]
    fn surrounding_blanks_are_allowed_around_the_operand() {
        let mut env = Environment::new();
        assert!(matches!(
            builtin_exit(&mut env, &argv(&["exit", " 5 "])),
            Err(ShellError::Exit(5))
        ));
    }

    #[test]
    fn bare_exit_carries_the_last_status_out() {
        let mut env = Environment::new();
        env.last_status = 7;
        assert!(matches!(
            builtin_exit(&mut env, &argv(&["exit"])),
            Err(ShellError::Exit(7))
        ));
    }

    /// Only the low byte survives `wait`, so the shell has to report what the caller will see.
    #[test]
    fn an_out_of_range_status_is_folded_into_a_byte() {
        let mut env = Environment::new();
        assert!(matches!(
            builtin_exit(&mut env, &argv(&["exit", "300"])),
            Err(ShellError::Exit(44))
        ));
        assert!(matches!(
            builtin_exit(&mut env, &argv(&["exit", "-1"])),
            Err(ShellError::Exit(255))
        ));
        assert!(matches!(
            builtin_return(&mut env, &argv(&["return", "300"])),
            Err(ShellError::Return(44))
        ));
    }

    /// A second operand is a usage error, and a usage error is not a reason to end the shell.
    #[test]
    fn too_many_operands_does_not_exit() {
        let mut env = Environment::new();
        assert_eq!(
            builtin_exit(&mut env, &argv(&["exit", "1", "2"])).expect("no exit"),
            1
        );
    }
}

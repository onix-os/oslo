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
/// How a name resolves, for the other builtins that report it rather than run it.
///
/// `type` and `which` answer the same question in different words, and a shell where they disagree
/// has two dispatch tables and one of them is wrong. There is one, and it is [`resolve::ways`].
pub use resolve::{Kind, ways};
/// Render a function definition as shell source; also what `set` and `declare -f` need to print.
pub use unparse::format_function;

use crate::env::origin_now;
use crate::env::scope::Environment;
use crate::exec::eval_command_list;
use oslo_base::error::{Result, ShellError};
use std::fs;

/// Parse a builtin's numeric operand the way bash's `legal_number` does.
///
/// Surrounding blanks are allowed (`exit " 5 "` is 5 in bash) but nothing else is: a name that
/// merely *starts* with digits is not a number, or `exit 1x` would silently exit 1.
fn numeric_operand<T: std::str::FromStr>(name: &str, raw: &str) -> std::result::Result<T, ()> {
    raw.trim().parse::<T>().map_err(|_| {
        eprintln!(
            "{}{}: {}: numeric argument required",
            origin_now(),
            name,
            raw
        );
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
                eprintln!(
                    "{}{}: {}: numeric argument required",
                    origin_now(),
                    name,
                    raw
                );
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
        // **Clamped to the loops that are actually there.** POSIX: when `n` is greater than the
        // number of enclosing loops, the outermost one is exited. Re-raised undecremented past the
        // last loop it would instead escape the script — `for i in 1 2; do break 2; done; echo after`
        // printed nothing, having abandoned everything after the loop.
        Ok(n) => Err(ShellError::Break(n.min(env.loops()))),
        Err(code) => Ok(code),
    }
}

/// `continue [n]` — start the next iteration of the `n`th enclosing loop.
pub fn builtin_continue(env: &mut Environment, args: &[String]) -> Result<i32> {
    match loop_depth("continue", args) {
        Ok(_) if !env.in_loop() => Ok(0),
        // The same clamp, and the same reason — see `builtin_break`.
        Ok(n) => Err(ShellError::Continue(n.min(env.loops()))),
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
        eprintln!("{}exit: too many arguments", origin_now());
        // Not an exit: bash refuses the request and leaves the shell running.
        return Ok(1);
    }
    if refuse_over_stopped_jobs(env) {
        return Ok(1);
    }
    let code = match args.get(1) {
        // Bare `exit` carries the last command's status out, so `cmd; exit` is `cmd; exit $?` —
        // **except inside the EXIT trap**, where it carries out the status the shell was already
        // leaving with. The trap runs *after* that status is decided, so letting its own last
        // command decide instead would let a cleanup step rewrite the result of the whole script.
        //
        // `trap 'stty …; exit' 0` is the idiom that shows it: `/usr/bin/bzmore` restores the
        // terminal on the way out, and with no terminal to restore the `stty` fails — which turned
        // a successful run into exit 1. bash and dash both report 0. An explicit operand still
        // wins, so `trap 'exit 7' 0` leaves with 7.
        None => super::process::exit_trap_status().unwrap_or(env.last_status),
        Some(raw) => match numeric_operand::<i64>("exit", raw) {
            Ok(n) => n as i32,
            Err(()) => return Err(ShellError::Exit(2)),
        },
    };
    Err(ShellError::Exit(exit_status(code)))
}

/// Whether this `exit` should be refused because there are stopped jobs, and say so if it is.
///
/// **The first `exit` warns and stays; a second one leaves.** bash's behaviour, and it exists
/// because a stopped job is invisible: leaving without a word means the work is either killed or
/// silently orphaned, and either way the person did not decide it. Asking twice is the whole
/// mechanism — the warning is the reminder, and repeating the command is the confirmation.
///
/// Interactive shells only. A script has nobody to warn and must not stop for one, and it is where
/// a refusal would be a hang rather than a prompt.
///
/// **This mattered more once `oslo.misc.interrupt_escape` existed.** Ctrl-Z is deliberate, so
/// somebody who stopped a job knows it is there; a job stopped by the escalation is one the shell
/// stopped *for* you, and that is exactly the job you would otherwise walk away from.
fn refuse_over_stopped_jobs(env: &mut Environment) -> bool {
    if !env.interactive() || !crate::exec::job::job_control_active() {
        return false;
    }
    // Cleared by every other command, so the confirmation has to be the *next* thing typed —
    // `exit`, a command, `exit` asks again, which is what makes it a confirmation rather than a
    // flag that drifts.
    if env.take_exit_warned() {
        return false;
    }
    let stopped = crate::exec::job::with_jobs(|jobs| {
        jobs.jobs()
            .iter()
            .filter(|job| matches!(job.state, crate::exec::job::JobState::Stopped))
            .count()
    });
    if stopped == 0 {
        return false;
    }
    eprintln!("{}exit: there are stopped jobs", origin_now());
    env.note_exit_warned();
    true
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

    // `--` ends the options, and `eval` has none — but it still has to be *consumed*, or it
    // becomes the first word of the code and the shell looks for a command called `--`.
    // atuin's widget dispatch is written `builtin eval -- "$widget"`, and without this every
    // keybinding it installs answered `oslo: --: command not found`.
    let operands = match args.get(1).map(String::as_str) {
        Some("--") => &args[2..],
        _ => &args[1..],
    };
    if operands.is_empty() {
        return Ok(0);
    }

    let code = operands.join(" ");

    // `x='eval "$x"'; eval "$x"` recurses through the parser and the evaluator with no function
    // call to bound it, so `eval` carries the same nesting counter `source` does.
    env.enter_nested_script()?;
    let result =
        match crate::syntax::parse_with_aliases(&code, !env.get_aliases().is_empty(), &|n| {
            env.get_alias(n).map(str::to_string)
        }) {
            Ok(ast) => eval_command_list(env, &ast),
            // A syntax error in evaluated text is the *builtin's* failure, not the script's:
            // bash reports it, gives `eval` status 2, and carries on with the next command.
            //
            // **Under `--posix` it is fatal instead**, because `eval` is a special builtin and
            // POSIX 2.8.1 ends a non-interactive shell on a utility error in one. Raised rather
            // than decided here: `posix::resolve_builtin_result` folds it back to this same status
            // for a shell that is not in POSIX mode, so the ordinary case is unchanged. Measured —
            // `bash --posix -c 'eval "if"; echo ALIVE'` prints nothing and exits 2.
            Err(e) => {
                eprintln!("{}eval: {}", origin_now(), e);
                Err(oslo_base::error::ShellError::utility_error("eval", 2))
            }
        };
    env.exit_nested_script();
    result
}

pub fn builtin_source(env: &mut Environment, args: &[String]) -> Result<i32> {
    if args.len() < 2 {
        eprintln!("{}source: filename argument required", origin_now());
        return Ok(1);
    }

    let file_path = &args[1];
    let content = match fs::read_to_string(file_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!(
                "oslo: source: {}: {}",
                file_path,
                oslo_base::error::reason(&e)
            );
            // `.` is a special builtin, so a file it cannot read ends a non-interactive shell in
            // POSIX mode — `bash --posix -c '. /nonexistent; echo ALIVE'` prints nothing. Outside
            // POSIX this folds back to status 1 and the script carries on, as it always did.
            return Err(oslo_base::error::ShellError::utility_error("source", 1));
        }
    };

    // A file that sources itself re-enters the parser and the evaluator once per level. The
    // counter is entered only after the file is known to be readable, so a missing file still
    // costs nothing, and exited on every path out so a `return` cannot leave it drifting.
    env.enter_nested_script()?;
    // `$FUNCNAME`'s outermost entry, as bash spells it: a function called from a sourced file
    // reads `f source`. `eval` gets none, and bash gives it none either.
    env.enter_script_frame("source");
    // And the file itself, so a failure inside it names it rather than the script that sourced it.
    env.enter_source_file(file_path);
    let result =
        match crate::syntax::parse_with_aliases(&content, !env.get_aliases().is_empty(), &|n| {
            env.get_alias(n).map(str::to_string)
        }) {
            Ok(ast) => eval_command_list(env, &ast),
            // As with `eval`: the sourced file failing to parse leaves `source` with status 2 and
            // the calling script still running.
            Err(e) => {
                eprintln!("{}{}: {}", origin_now(), file_path, e);
                Ok(2)
            }
        };
    env.exit_source_file();
    env.exit_script_frame();
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
    use oslo_base::error::ShellError;

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

#[cfg(test)]
mod depth_tests {
    use super::{builtin_break, builtin_continue};
    use crate::env::Environment;
    use oslo_base::error::ShellError;

    fn argv(words: &[&str]) -> Vec<String> {
        words.iter().map(|s| s.to_string()).collect()
    }

    /// **`break n` past the last loop exits the outermost one, it does not abandon the script.**
    /// POSIX says so, and undecremented the signal escaped every enclosing list — so
    /// `for i in 1 2; do break 2; done; echo after` printed nothing at all.
    #[test]
    fn a_depth_past_the_last_loop_is_clamped_to_it() {
        let mut env = Environment::new();
        env.enter_loop();
        match builtin_break(&mut env, &argv(&["break", "5"])) {
            Err(ShellError::Break(n)) => assert_eq!(n, 1, "clamped to the one loop there is"),
            other => panic!("expected a Break, got {other:?}"),
        }
        match builtin_continue(&mut env, &argv(&["continue", "9"])) {
            Err(ShellError::Continue(n)) => assert_eq!(n, 1),
            other => panic!("expected a Continue, got {other:?}"),
        }
    }

    /// A depth the loops can honour is passed through untouched, so nesting still peels.
    #[test]
    fn a_depth_the_loops_can_honour_is_left_alone() {
        let mut env = Environment::new();
        env.enter_loop();
        env.enter_loop();
        match builtin_break(&mut env, &argv(&["break", "2"])) {
            Err(ShellError::Break(n)) => assert_eq!(n, 2),
            other => panic!("expected a Break, got {other:?}"),
        }
    }
}

//! Evaluating a simple command.
//!
//! Expansion, alias substitution, command-prefix assignments, then command search: the order in
//! which a word becomes an alias, a function, a builtin or a program on PATH.
//!
//! The submodules are the parts that answer a question of their own: `external` runs a program
//! once it has been found, `assign` decides what each shape of assignment means, `trace` renders
//! `set -x`, `autocd` owns the `shopt -s autocd` option, and `posix` owns everything POSIX mode
//! changes about a command's *outcome*.

mod assign;
mod autocd;
mod external;
mod posix;
mod trace;

pub use autocd::set_autocd;

use crate::ast::*;
use crate::env::Environment;
use crate::error::{Result, ShellError};
use crate::exec::pipeline::eval_command;
use crate::exec::redirect::RedirectGuard;
use crate::exec::simple::external::{Lookup, look_up_command, run_external};
use crate::expand::{expand_word, expand_word_to_string};
pub(crate) fn eval_simple_command(env: &mut Environment, simple: &SimpleCommand) -> Result<i32> {
    // Any `<(cmd)` in this command's words opens a pipe during expansion; it has to stay open
    // until the command has run and be closed afterwards, whichever way the command ends. The
    // wrapper is what guarantees the second half on the error paths too.
    let result = eval_simple_command_inner(env, simple);
    if !env.procsubs.is_empty() {
        let mut open = std::mem::take(&mut env.procsubs);
        crate::exec::procsub::finish(&mut open);
    }
    result
}

fn eval_simple_command_inner(env: &mut Environment, simple: &SimpleCommand) -> Result<i32> {
    // R6.5: a signal that arrived while the previous command ran is handled *between* commands,
    // where the trap body can run ordinary shell code. See `run_pending_traps`.
    crate::env::builtins::run_pending_traps(env)?;

    // Before expansion: `$BASH_COMMAND` is what is about to run, not what it turns into.
    crate::env::builtins::run_debug_trap(env, &crate::ast::render::simple_command(simple));

    if simple.words.is_empty() {
        return apply_assignments_only(env, simple);
    }

    let mut words = Vec::new();
    for w in &simple.words {
        words.extend(expand_word(env, w)?);
    }

    if words.is_empty() {
        return apply_assignments_only(env, simple);
    }

    // Aliases are *not* expanded here. Substitution happens on the source text before it is
    // parsed — see [`crate::parser::alias`] — because an alias body is source, not a list of
    // arguments, and only a pre-parse pass can let `alias forever='while :; do'` work. Doing it
    // in both places would also expand twice: `alias ls='ls -F'` would have become `ls -F -F`.

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
        // A prefix assignment lasts exactly as long as the command, and oslo undoes it by
        // restoring a *scalar* from the scope frame. `a=(1 2) cmd` and `a[1]=x cmd` have no such
        // undo, so they are refused rather than left behind after the command finishes.
        let (AssignmentTarget::Name(name), AssignmentValue::Scalar(value)) =
            (&assign.target, &assign.value)
        else {
            return Err(ShellError::SyntaxError(format!(
                "{}: an array assignment cannot be a command prefix",
                assign.name()
            )));
        };
        let val_str = expand_word_to_string(env, value)?;
        if is_declaration {
            words.push(format!("{}={}", name, val_str));
        } else {
            prefix_assignments.push((name.clone(), val_str));
        }
    }

    trace::trace_command(env, &prefix_assignments, &words);

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

/// Run a command that has no command word: `x=1`, or `x=1 $empty`.
///
/// Shared by the two paths that reach it — assignments written with no word at all, and
/// assignments whose word expanded to nothing — because they have to agree about the two things
/// that are easy to get subtly different: the assignment is still performed, and `set -x` still
/// traces it.
///
/// An assignment that the environment *refused* — a read-only name, or one `environ` cannot hold
/// — is a failed command. This used to answer `Ok(0)` regardless: `set_var` printed
/// `r: is read only`, returned `false`, and the `false` was dropped on the floor, so `r=2; echo
/// $?` said the assignment had worked (PLAN C3).
fn apply_assignments_only(env: &mut Environment, simple: &SimpleCommand) -> Result<i32> {
    let mut assignments = Vec::with_capacity(simple.assignments.len());
    let mut refused: Option<String> = None;
    for assign in &simple.assignments {
        // Applied before the next one is expanded, not batched at the end: POSIX 2.9.1 evaluates
        // assignments left to right and makes each visible to the next, so `a=1 b=${a}2` sets
        // `b` to `12`. What each shape means is `assign`'s business.
        let outcome = assign::apply_assignment(env, assign)?;
        if !outcome.assigned && refused.is_none() {
            refused = Some(assign.name().to_string());
        }
        assignments.push((assign.name().to_string(), outcome.trace));
    }
    // Traced first: `set -x` shows what the shell tried to do, and the refusal is on stderr
    // beside it. The first name to fail is the one reported, as bash reports the first.
    trace::trace_command(env, &assignments, &[]);
    if let Some(name) = refused {
        return posix::assignment_failure(env, &name);
    }
    Ok(apply_wordless_redirections(env, &simple.redirections))
}

/// Run an argv that is already expanded, as Lua's `oslo.run` hands it over.
///
/// This is the whole point of the argv call model: no word, no quoting, no glob and no field
/// splitting stand between the caller's list and the command. `oslo.run{"rm", name}` cannot
/// misbehave for a `name` with a space or a `*` in it, where `oslo.proc.exec("rm " .. name)` can.
///
/// Everything past this point is shared with the shell — the same command search, the same
/// builtins, the same functions — so `sh.cd("/tmp")` moves the shell exactly as `cd /tmp` does.
pub fn run_argv(env: &mut Environment, argv: &[String]) -> Result<i32> {
    let Some(name) = argv.first() else {
        return Ok(0);
    };
    crate::env::builtins::run_pending_traps(env)?;
    run_command_word(env, name, argv, &[])
}

/// Dispatch an already-expanded command word.
///
/// POSIX 2.9.1.1 command search, and the order matters at every step:
///
/// 1. **alias** — done by the caller, before the word is even split.
/// 2. **special builtin**, in POSIX mode only. POSIX puts `export`, `eval`, `set`, `.` and the
///    rest ahead of functions; bash follows that only under `--posix`, where it goes further and
///    refuses to *define* such a function at all.
/// 3. **function**. This is the step oslo skipped: `is_builtin` was consulted first, so
///    `echo() { … }`, `cd() { … }` and `test() { … }` could be defined but never called, and
///    `type echo` insisted it was a builtin. Wrapping a builtin is how a shell script overrides
///    behaviour it does not control, and silently ignoring the wrapper runs the original.
/// 4. **regular builtin**.
/// 5. **PATH**, or a path operand — see the `external` submodule.
fn run_command_word(
    env: &mut Environment,
    cmd_name: &str,
    words: &[String],
    redirections: &[Redirection],
) -> Result<i32> {
    let name = cmd_name.trim();

    if posix::special_builtins_outrank_functions(env)
        && crate::env::scope::is_special_builtin(name)
        && env.is_builtin(name)
    {
        return run_builtin(env, name, words, redirections);
    }

    if let Some(func_body) = env.get_function(name).cloned() {
        return call_function_command(env, &func_body, words, redirections);
    }

    if env.is_builtin(name) {
        return run_builtin(env, name, words, redirections);
    }

    run_program(env, name, words, redirections)
}

/// Apply the command's redirections and run it as a builtin.
///
/// A builtin never sees its own redirections, so the one thing decided here is how long they
/// last. `exec > "$log" 2>&1` — `exec` with no command word — is the form POSIX says applies to
/// the shell itself from then on, so its guard must not restore anything; every other builtin
/// gets the ordinary guard that puts the descriptors back when the command ends.
///
/// Both outcomes are then passed through `posix`, which is where a *special* builtin's utility
/// error or redirection error becomes the end of a POSIX-mode shell.
fn run_builtin(
    env: &mut Environment,
    name: &str,
    words: &[String],
    redirections: &[Redirection],
) -> Result<i32> {
    let mut guard = if crate::env::builtins::exec_makes_redirections_permanent(name, words) {
        RedirectGuard::for_exec()
    } else {
        RedirectGuard::new()
    };
    if let Err(e) = guard.apply(env, redirections) {
        return posix::redirect_failure(env, name, report_redirect_failure(&e));
    }
    let result = execute_builtin(env, name, words);
    posix::resolve_builtin_result(env, name, result)
}

/// Call a shell function, absorbing the control flow that must not escape it.
fn call_function_command(
    env: &mut Environment,
    body: &Command,
    words: &[String],
    redirections: &[Redirection],
) -> Result<i32> {
    // Checked before anything is set up, so a refused call has nothing to unwind. `f() { f; }`
    // recurses through the whole evaluator; without this the stack overflows and Rust aborts
    // the process outright, status 134 and a core dump.
    env.enter_function()?;
    let res = call_function(env, body, words, redirections);
    env.exit_function();

    // `return` unwinds to here and becomes the function's exit status. `break`/`continue`
    // are also absorbed: they must not escape into a loop in the caller.
    match res {
        Err(ShellError::Return(code)) => Ok(code),
        Err(ShellError::Break(_)) | Err(ShellError::Continue(_)) => Ok(0),
        other => other,
    }
}

/// Run an external program, or explain why the word does not name one.
///
/// The statuses are the ones a caller reads: 127 for "no such command", 126 for "found it,
/// cannot run it". oslo used to report 127 for a non-executable file and, for a directory,
/// nothing at all — it changed directory and returned 0 (PLAN R5.13).
fn run_program(
    env: &mut Environment,
    cmd_name: &str,
    words: &[String],
    redirections: &[Redirection],
) -> Result<i32> {
    match look_up_command(cmd_name) {
        Lookup::Program(path) => run_external(env, &path, cmd_name, words, redirections),
        Lookup::Directory => match autocd::try_autocd(env, cmd_name, words) {
            Some(result) => result,
            None => report_unrunnable(env, redirections, cmd_name, "Is a directory", 126),
        },
        Lookup::NotExecutable => {
            report_unrunnable(env, redirections, cmd_name, "Permission denied", 126)
        }
        // A bare word is a PATH search, so a directory of that name in the *current* directory
        // was never a candidate; it only becomes one if autocd is on. bash reports the same
        // "command not found" and 127 here even with `shopt -s autocd` set, as long as the shell
        // is not interactive.
        Lookup::NotFound => match autocd::try_autocd(env, cmd_name, words) {
            Some(result) => result,
            // Before giving up, ask the config. A distribution's package manager is the obvious
            // handler — "nvim is in package neovim", or install it and run it — and a handler that
            // resolved the situation answers with the status to report. Everyone else bolts this
            // on as a shell function; here it is a hook.
            None => match crate::lua::engine::ask_hook_here(
                "command-not-found",
                vec![crate::lua::eval::value::Value::str(cmd_name)],
            ) {
                Some(status) => Ok(status),
                None => {
                    // Nobody handled it, so say what a person needs next: the name that was
                    // probably meant. Only when the shell is interactive — a script's stderr is
                    // read by machines, and bash says exactly "command not found" there.
                    let hint = if env.interactive() {
                        let path = env.get_var("PATH").unwrap_or_default().to_string();
                        crate::interactive::command_index::nearest(&path, cmd_name)
                    } else {
                        None
                    };
                    match hint {
                        Some(near) => report_unrunnable(
                            env,
                            redirections,
                            cmd_name,
                            &format!("command not found; did you mean {near}?"),
                            127,
                        ),
                        None => {
                            report_unrunnable(env, redirections, cmd_name, "command not found", 127)
                        }
                    }
                }
            },
        },
    }
}

/// Report a command word that could not be run, with the command's own redirections in force.
///
/// The diagnostic belongs to the *command*, not to the shell, so it goes wherever the command
/// pointed its stderr: `nosuchcommand 2>/dev/null` is silent in every shell, and that is the
/// shape of every feature probe written before `command -v` existed. oslo printed to the shell's
/// own stderr instead, so a script full of such probes filled the terminal with noise it had
/// explicitly asked to discard.
fn report_unrunnable(
    env: &mut Environment,
    redirections: &[Redirection],
    cmd_name: &str,
    reason: &str,
    status: i32,
) -> Result<i32> {
    let mut guard = RedirectGuard::new();
    if let Err(e) = guard.apply(env, redirections) {
        // The redirection failed too. That is the failure the user has to fix first, and it is
        // the one bash reports here as well.
        return Ok(report_redirect_failure(&e));
    }
    eprintln!("oslo: {}: {}", cmd_name, reason);
    Ok(status)
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

/// Report a redirection failure and hand back the status the failed command takes on.
///
/// A redirection that cannot be set up fails *the command*, not the shell. oslo used to propagate
/// the error to `main`, which exited — so `echo hi < /nonexistent; echo CONTINUE` never printed
/// CONTINUE, while the same redirection on an external command continued happily. The two paths
/// disagreed with each other; this is the one place that decides.
///
/// Status 1, measured against `bash --posix` for a builtin (`read x < /nonexistent`), a bad
/// descriptor (`echo hi >&7`), a function, a compound and an external command: all print a
/// diagnostic, set `$?` to 1 and carry on.
///
/// The one case bash treats differently is a redirection error on a *special* builtin (`:`,
/// `export`, …) in POSIX mode, which does abort the shell. That is now implemented, but not
/// here: this function only reports and scores the failure, and [`posix::redirect_failure`]
/// decides whether the shell survives it. Callers on a path that cannot be a special builtin —
/// a function, a compound, an external command, a command word with no builtin behind it — use
/// this answer directly, because for them the question never arises.
pub(crate) fn report_redirect_failure(err: &ShellError) -> i32 {
    eprintln!("oslo: {}", err);
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

/// Run `cmd_name` as a builtin, assuming redirections are already in place.
///
/// The only dispatcher, and it owns no list of its own: it asks the registry. The `match` that
/// used to be here named 30 builtins and their functions a second time, so the registry could
/// hold a *different* implementation for a name and never be consulted — which is exactly what
/// made `oslo.register_builtin('echo', …)` do nothing (PLAN R5.6, R9.8) — while the `_` arm
/// answered `Ok(0)` for anything unlisted, turning a name that was a builtin only according to
/// `is_builtin` into a command that silently succeeded without doing anything.
///
/// Public to the crate so `command` and `builtin` (PLAN R5.7) can reach a builtin without
/// re-entering command search.
pub(crate) fn execute_builtin(
    env: &mut Environment,
    cmd_name: &str,
    words: &[String],
) -> Result<i32> {
    match env.exec_custom_builtin(cmd_name, words) {
        Some(result) => result,
        // Only reachable from a caller that dispatched without asking `is_builtin` first, which
        // is a bug here rather than in the user's script — but it is still the user who gets the
        // message, so it says what a shell says about a name it cannot run.
        None => {
            eprintln!("oslo: {}: not a shell builtin", cmd_name);
            Ok(127)
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::env::Environment;

    /// Run a snippet in a fresh environment and hand back the environment to inspect.
    ///
    /// `Environment::new()` snapshots the *process* environment and `export` writes back into it,
    /// so an exported name set by one test is visible to every environment built afterwards.
    /// Tests here therefore use names unique to each test rather than a shared `v`.
    fn run(src: &str) -> Environment {
        let mut env = Environment::new();
        run_in(&mut env, src).expect("exec");
        env
    }

    /// Run a snippet in an environment the caller has already configured, and hand back the
    /// result rather than unwrapping it — the POSIX-mode cases are about the error itself.
    fn run_in(env: &mut Environment, src: &str) -> crate::error::Result<i32> {
        let script = crate::parser::parse_bash_script(src).expect("parse");
        crate::exec::eval_command_list(env, &script)
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
        assert_eq!(var("oslo_g1=Cargo.*", "oslo_g1"), "Cargo.*");
        assert_eq!(var("oslo_g2=Cargo.* true", "oslo_g2"), "");
        assert_eq!(var("export oslo_g3=Cargo.*", "oslo_g3"), "Cargo.*");
    }

    #[test]
    fn assignment_rhs_is_not_field_split() {
        assert_eq!(var("IFS=:\noslo_s1=a:b:c", "oslo_s1"), "a:b:c");
        assert_eq!(var("IFS=:\nexport oslo_s2=a:b:c", "oslo_s2"), "a:b:c");
        // Interior whitespace from an unquoted expansion survives too.
        assert_eq!(var("oslo_s3='a  b'\noslo_s4=$oslo_s3", "oslo_s4"), "a  b");
    }

    // The third leg of R2.9 — `x=$(printf 'a\nb')` keeps its newline — is deliberately *not*
    // tested here. Command substitution forks (`exec/substitution.rs`), and libtest runs unit
    // tests on a pool of threads: a child forked out of a multi-threaded process inherits any
    // mutex another thread happened to hold, so the child deadlocks in the allocator before it
    // can write to the pipe and the parent blocks forever in `waitpid`. That is a property of
    // the harness, not of the shell (oslo itself is single-threaded), so the case lives in
    // `tests/expansion_tests.rs`, which spawns the real binary.

    /// The `words.is_empty()` fallback path — a command word that expands to nothing leaves only
    /// the assignments — must apply the same rule as the ordinary one.
    #[test]
    fn assignment_survives_an_empty_command_word() {
        assert_eq!(
            var("IFS=:\noslo_e1=\noslo_e2=a:b $oslo_e1", "oslo_e2"),
            "a:b"
        );
    }

    /// A prefix assignment is scoped to its command and must not leak back out.
    #[test]
    fn prefix_assignment_does_not_outlive_its_command() {
        assert_eq!(run("oslo_p1=a:b true").get_var("oslo_p1"), None);
    }

    /// A function must be found before the builtin of the same name (PLAN R5.6). Asserted on a
    /// side effect rather than on stdout, because the unit-test harness captures the shell's
    /// `println!` but not what a builtin writes; if the wrapper never ran, the variable is unset.
    #[test]
    fn a_function_shadows_a_regular_builtin() {
        let env = run("cd() { oslo_shadow=called; }\ncd /nonexistent-dir");
        assert_eq!(env.get_var("oslo_shadow"), Some("called"));
    }

    /// …including the ones whose names the dispatcher used to hardcode.
    #[test]
    fn a_function_shadows_echo_and_test() {
        let env = run("echo() { oslo_shadow_echo=1; }\necho hi");
        assert_eq!(env.get_var("oslo_shadow_echo"), Some("1"));
        let env = run("test() { oslo_shadow_test=1; }\ntest -f /etc/hosts");
        assert_eq!(env.get_var("oslo_shadow_test"), Some("1"));
    }

    /// POSIX mode is the only mode in which a special builtin outranks a function.
    ///
    /// The mode lives on the `Environment` now, not in a process-global that every other test in
    /// this binary had to be protected from: the second half here simply runs in a different
    /// environment, and nothing has to be restored afterwards.
    #[test]
    fn posix_mode_puts_special_builtins_ahead_of_functions() {
        let env = run("export() { oslo_shadow_export=1; }\nexport oslo_ignored=x");
        assert_eq!(env.get_var("oslo_shadow_export"), Some("1"));

        let mut env = Environment::new();
        env.set_option(crate::env::options::ShellOption::Posix, true);
        run_in(
            &mut env,
            "export() { oslo_shadow_export2=1; }\nexport oslo_special=y",
        )
        .expect("exec");
        assert_eq!(env.get_var("oslo_shadow_export2"), None);
        assert_eq!(env.get_var("oslo_special"), Some("y"));
    }

    /// C3: a refused assignment is a *failed* command, not a silent success.
    ///
    /// Outside POSIX mode the shell carries on with status 1, which is what
    /// `bash -c $'readonly r=1\nr=2\necho "$?"\necho after'` does.
    #[test]
    fn a_refused_assignment_fails_the_command_and_the_shell_carries_on() {
        let mut env = Environment::new();
        env.set_var("oslo_ro_a", "1", false);
        env.set_readonly("oslo_ro_a");
        assert_eq!(run_in(&mut env, "oslo_ro_a=2").expect("not fatal"), 1);
        assert_eq!(env.get_var("oslo_ro_a"), Some("1"));
        // …and the next command still runs.
        assert_eq!(run_in(&mut env, "oslo_ro_b=ok").expect("not fatal"), 0);
        assert_eq!(env.get_var("oslo_ro_b"), Some("ok"));
    }

    /// …and in POSIX mode the same assignment ends the shell, as POSIX 2.8.1 requires and
    /// `bash --posix -c` does. It unwinds as `exit`, so the EXIT trap still runs.
    #[test]
    fn a_refused_assignment_exits_a_posix_shell() {
        let mut env = Environment::new();
        env.set_option(crate::env::options::ShellOption::Posix, true);
        env.set_var("oslo_ro_c", "1", false);
        env.set_readonly("oslo_ro_c");
        match run_in(&mut env, "oslo_ro_c=2\noslo_never=reached") {
            Err(crate::error::ShellError::Exit(status)) => {
                assert_eq!(status, crate::error::FATAL_EXIT_STATUS);
            }
            other => panic!("expected the shell to exit, got {:?}", other),
        }
        assert_eq!(env.get_var("oslo_never"), None);
    }

    /// An ordinary non-zero status must *not* end a POSIX shell, even from a special builtin.
    /// `bash --posix -c 'shift 5; echo alive'` prints `alive` and exits 0; a check written as
    /// `status != 0` would have killed the shell here.
    #[test]
    fn a_failing_special_builtin_that_is_not_a_utility_error_is_survivable() {
        let mut env = Environment::new();
        env.set_option(crate::env::options::ShellOption::Posix, true);
        assert_eq!(
            run_in(&mut env, "shift 5\noslo_still_alive=yes").expect("not fatal"),
            0
        );
        assert_eq!(env.get_var("oslo_still_alive"), Some("yes"));
    }

    /// Every builtin now dispatches through the registry, so a name the registry does not have
    /// is not a builtin at all — it used to reach the `_` arm and return 0 without running.
    #[test]
    fn an_unregistered_name_is_not_a_builtin() {
        let env = Environment::new();
        assert!(!env.is_builtin("oslo-not-a-builtin"));
        assert!(env.is_builtin("type"));
    }
}

//! Evaluating a simple command.
//!
//! Expansion, alias substitution, command-prefix assignments, then command search: the order in
//! which a word becomes an alias, a function, a builtin or a program on PATH. Running the
//! program once it has been found is the `external` submodule's job.

mod assign;
mod external;
mod trace;

use crate::ast::*;
use crate::env::Environment;
use crate::env::scope::is_special_builtin;
use crate::error::{Result, ShellError};
use crate::exec::pipeline::eval_command;
use crate::exec::redirect::RedirectGuard;
use crate::exec::simple::external::{Lookup, look_up_command, run_external};
use crate::expand::{expand_word, expand_word_to_string};
use crate::lexer::Lexer;
use std::sync::atomic::{AtomicBool, Ordering};

/// Whether a command word that names a directory should be treated as `cd` to it.
///
/// bash's `shopt -s autocd`, and off for the same reason bash has it off: in a script `build`
/// means "run the build command", and silently changing directory instead makes every later
/// relative path in that script resolve somewhere else, with status 0 to say all is well
/// (PLAN R5.13). Process-global rather than a field on [`Environment`], like
/// [`crate::exec::pipeline::set_interactive`]: it is a property of the invocation, and a forked
/// child inherits it as it stands.
///
/// autocd is a `shopt` option, and `shopt` still does not exist — `set -o` (PLAN R6.1) covers
/// only the POSIX option set, which autocd is not part of. [`set_autocd`] stays the hook a
/// future `shopt` will call, and the `RUSH_AUTOCD` shell variable is how a user opts in today.
static AUTOCD: AtomicBool = AtomicBool::new(false);

/// Whether this shell is in POSIX mode, which decides only one thing here: whether a special
/// builtin is found before a shell function, as POSIX 2.9.1.1 requires and bash does only
/// under `--posix`.
static POSIX_MODE: AtomicBool = AtomicBool::new(false);

/// Enable or disable autocd: whether a command word naming a directory means `cd` to it.
///
/// Off by default, and effective only in an interactive shell. This is the hook `shopt -s
/// autocd` will call once `shopt` exists; the `set -o` table in `crate::env::options` does not
/// carry it, because bash does not either.
pub fn set_autocd(yes: bool) {
    AUTOCD.store(yes, Ordering::Relaxed);
}

/// Declare whether this shell is in POSIX mode (`--posix`, `set -o posix`).
pub fn set_posix_mode(yes: bool) {
    POSIX_MODE.store(yes, Ordering::Relaxed);
}

/// Whether this shell is in POSIX mode; see [`set_posix_mode`].
pub fn posix_mode() -> bool {
    POSIX_MODE.load(Ordering::Relaxed)
}

pub(crate) fn eval_simple_command(env: &mut Environment, simple: &SimpleCommand) -> Result<i32> {
    // R6.5: a signal that arrived while the previous command ran is handled *between* commands,
    // where the trap body can run ordinary shell code. See `run_pending_traps`.
    crate::env::builtins::run_pending_traps(env)?;

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
        // A prefix assignment lasts exactly as long as the command, and rush undoes it by
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
fn apply_assignments_only(env: &mut Environment, simple: &SimpleCommand) -> Result<i32> {
    let mut assignments = Vec::with_capacity(simple.assignments.len());
    for assign in &simple.assignments {
        // Applied before the next one is expanded, not batched at the end: POSIX 2.9.1 evaluates
        // assignments left to right and makes each visible to the next, so `a=1 b=${a}2` sets
        // `b` to `12`. What each shape means is `assign`'s business.
        let value = assign::apply_assignment(env, assign)?;
        assignments.push((assign.name().to_string(), value));
    }
    trace::trace_command(env, &assignments, &[]);
    Ok(apply_wordless_redirections(env, &simple.redirections))
}

/// Dispatch an already-expanded command word.
///
/// POSIX 2.9.1.1 command search, and the order matters at every step:
///
/// 1. **alias** — done by the caller, before the word is even split.
/// 2. **special builtin**, in POSIX mode only. POSIX puts `export`, `eval`, `set`, `.` and the
///    rest ahead of functions; bash follows that only under `--posix`, where it goes further and
///    refuses to *define* such a function at all.
/// 3. **function**. This is the step rush skipped: `is_builtin` was consulted first, so
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

    if posix_mode() && is_special_builtin(name) && env.is_builtin(name) {
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
        return Ok(report_redirect_failure(&e));
    }
    execute_builtin(env, name, words)
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
/// cannot run it". rush used to report 127 for a non-executable file and, for a directory,
/// nothing at all — it changed directory and returned 0 (PLAN R5.13).
fn run_program(
    env: &mut Environment,
    cmd_name: &str,
    words: &[String],
    redirections: &[Redirection],
) -> Result<i32> {
    match look_up_command(cmd_name) {
        Lookup::Program(path) => run_external(env, &path, cmd_name, words, redirections),
        Lookup::Directory => match try_autocd(env, cmd_name, words) {
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
        Lookup::NotFound => match try_autocd(env, cmd_name, words) {
            Some(result) => result,
            None => report_unrunnable(env, redirections, cmd_name, "command not found", 127),
        },
    }
}

/// Report a command word that could not be run, with the command's own redirections in force.
///
/// The diagnostic belongs to the *command*, not to the shell, so it goes wherever the command
/// pointed its stderr: `nosuchcommand 2>/dev/null` is silent in every shell, and that is the
/// shape of every feature probe written before `command -v` existed. rush printed to the shell's
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
    eprintln!("rush: {}: {}", cmd_name, reason);
    Ok(status)
}

/// `cd` to `cmd_name` if this shell is allowed to guess that is what was meant.
///
/// `None` — the overwhelmingly common answer — leaves the caller to report the real failure.
/// Three conditions, all required: the shell is interactive (a person is there to see the
/// directory change and to have meant it), autocd is switched on, and the command is a bare
/// word with no arguments, since `build --release` is unambiguously not a `cd`.
fn try_autocd(env: &mut Environment, cmd_name: &str, words: &[String]) -> Option<Result<i32>> {
    if words.len() != 1 || !autocd_enabled(env) {
        return None;
    }
    if !std::path::Path::new(cmd_name).is_dir() {
        return None;
    }
    Some(crate::env::builtins::builtin_cd(
        env,
        &["cd".to_string(), cmd_name.to_string()],
    ))
}

/// Whether autocd may fire: interactive *and* opted in.
///
/// The interactive half is not configurable, in bash either — `bash -O autocd -c 'somedir'`
/// still reports `command not found`. A script's meaning must not depend on which directories
/// happen to exist beside it.
fn autocd_enabled(env: &Environment) -> bool {
    if !crate::exec::pipeline::is_interactive() {
        return false;
    }
    AUTOCD.load(Ordering::Relaxed)
        || env
            .get_var("RUSH_AUTOCD")
            .is_some_and(|v| !v.is_empty() && v != "0")
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

/// Run `cmd_name` as a builtin, assuming redirections are already in place.
///
/// The only dispatcher, and it owns no list of its own: it asks the registry. The `match` that
/// used to be here named 30 builtins and their functions a second time, so the registry could
/// hold a *different* implementation for a name and never be consulted — which is exactly what
/// made `rush.register_builtin('echo', …)` do nothing (PLAN R5.6, R9.8) — while the `_` arm
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
            eprintln!("rush: {}: not a shell builtin", cmd_name);
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

    /// A function must be found before the builtin of the same name (PLAN R5.6). Asserted on a
    /// side effect rather than on stdout, because the unit-test harness captures the shell's
    /// `println!` but not what a builtin writes; if the wrapper never ran, the variable is unset.
    #[test]
    fn a_function_shadows_a_regular_builtin() {
        let env = run("cd() { rush_shadow=called; }\ncd /nonexistent-dir");
        assert_eq!(env.get_var("rush_shadow"), Some("called"));
    }

    /// …including the ones whose names the dispatcher used to hardcode.
    #[test]
    fn a_function_shadows_echo_and_test() {
        let env = run("echo() { rush_shadow_echo=1; }\necho hi");
        assert_eq!(env.get_var("rush_shadow_echo"), Some("1"));
        let env = run("test() { rush_shadow_test=1; }\ntest -f /etc/hosts");
        assert_eq!(env.get_var("rush_shadow_test"), Some("1"));
    }

    /// POSIX mode is the only mode in which a special builtin outranks a function. The flag is
    /// process-global, so it is restored before the assertion that would otherwise leak it into
    /// every later test in this binary.
    #[test]
    fn posix_mode_puts_special_builtins_ahead_of_functions() {
        let env = run("export() { rush_shadow_export=1; }\nexport rush_ignored=x");
        assert_eq!(env.get_var("rush_shadow_export"), Some("1"));

        super::set_posix_mode(true);
        let env = run("export() { rush_shadow_export2=1; }\nexport rush_special=y");
        super::set_posix_mode(false);
        assert_eq!(env.get_var("rush_shadow_export2"), None);
        assert_eq!(env.get_var("rush_special"), Some("y"));
    }

    /// Every builtin now dispatches through the registry, so a name the registry does not have
    /// is not a builtin at all — it used to reach the `_` arm and return 0 without running.
    #[test]
    fn an_unregistered_name_is_not_a_builtin() {
        let env = Environment::new();
        assert!(!env.is_builtin("rush-not-a-builtin"));
        assert!(env.is_builtin("type"));
    }
}

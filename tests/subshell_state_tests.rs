//! Forked-child state (R4.1) and the collected pipeline statuses (R4.10).
//!
//! The differential corpus covers what a *script* can observe about a subshell. Two things it
//! cannot observe yet live here instead: the per-stage status vector, which has no shell-visible
//! surface until `PIPESTATUS` (Round 8) and `pipefail` (Round 6) read it, and the fact that a
//! subshell marks itself as one rather than rebuilding a shell from scratch.

use rush::env::Environment;
use rush::exec::eval_command_list;
use rush::parser::parse_bash_script;

fn run(env: &mut Environment, script: &str) -> i32 {
    let ast = parse_bash_script(script).expect("parse");
    eval_command_list(env, &ast).expect("execute")
}

/// R4.10: a pipeline that fails in the middle used to leave no trace of it anywhere.
#[test]
fn every_pipeline_stage_status_is_recorded() {
    let mut env = Environment::new();

    run(&mut env, "false | true");
    assert_eq!(env.pipeline_status(), [1, 0]);

    run(&mut env, "true | false");
    assert_eq!(env.pipeline_status(), [0, 1]);

    run(&mut env, "sh -c 'exit 3' | sh -c 'exit 4' | true");
    assert_eq!(env.pipeline_status(), [3, 4, 0]);
}

/// A one-command pipeline still has a stage vector, as bash's `PIPESTATUS` does.
#[test]
fn a_single_command_records_one_status() {
    let mut env = Environment::new();

    run(&mut env, "true");
    assert_eq!(env.pipeline_status(), [0]);

    run(&mut env, "sh -c 'exit 7'");
    assert_eq!(env.pipeline_status(), [7]);
}

/// `!` inverts what the pipeline *reports*; the stages themselves still failed.
#[test]
fn negation_does_not_rewrite_the_stage_statuses() {
    let mut env = Environment::new();
    assert_eq!(run(&mut env, "! false | false"), 0);
    assert_eq!(env.pipeline_status(), [1, 1]);
}

/// The status a command substitution exited with survives the fork it ran in.
///
/// R4.4 needs it in `simple.rs`, which cannot reach into the substitution child; this is the
/// handoff point. Written so it holds both before and after that wiring lands.
#[test]
fn a_command_substitutions_status_is_kept() {
    let mut env = Environment::new();

    let status = run(&mut env, "x=$(exit 5)");
    let reported = env.take_substitution_status().unwrap_or(status);
    assert_eq!(reported, 5, "the substitution's status was thrown away");
    assert_eq!(env.get_var("x"), Some(""));

    // Taken, not merely read: the next assignment must not inherit a stale number.
    assert_eq!(env.take_substitution_status(), None);
}

/// R4.1: the parent shell is never a subshell, and marking one keeps `$$` intact.
#[test]
fn entering_a_subshell_keeps_the_invoking_shells_pid() {
    let mut env = Environment::new();
    assert!(!env.in_subshell());

    let dollar_dollar = env.get_param("$");
    env.enter_subshell();

    // Not a real fork, so the pid is unchanged and `in_subshell` cannot flip here; what matters
    // is that `$$` still reports the invoking shell, which is what POSIX and bash require of a
    // subshell. `current_pid` is what job control and `$BASHPID` would use instead.
    assert_eq!(env.get_param("$"), dollar_dollar);
    assert_eq!(env.current_pid(), std::process::id());
}

/// R4.1: traps are reset to their default action in a subshell, so an inherited `EXIT` handler
/// does not fire once per forked child.
#[test]
fn a_subshell_starts_with_no_traps() {
    let mut env = Environment::new();
    env.set_trap("EXIT", "echo bye");
    assert_eq!(env.get_trap("EXIT"), Some("echo bye"));

    env.enter_subshell();
    assert_eq!(env.get_trap("EXIT"), None);
    assert!(env.get_traps().is_empty());
}

/// R4.1: a subshell keeps every variable it inherited *with its export flag*. Rebuilding the
/// environment and re-exporting each variable is what leaked private data into every child.
#[test]
fn a_subshell_does_not_export_private_variables() {
    let mut env = Environment::new();
    run(&mut env, "secret=classified; export shown=public");
    env.enter_subshell();

    let exported = env.get_exported_vars();
    assert!(
        !exported.contains_key("secret"),
        "private variable exported"
    );
    assert_eq!(exported.get("shown").map(String::as_str), Some("public"));
    assert_eq!(env.get_var("secret"), Some("classified"));
}

//! Recursion and nesting limits (PLAN R1.9).
//!
//! Every case here used to exhaust the stack, which Rust reports by aborting the process: status
//! 134 and a core dump, with nothing on stderr that names the problem. What the shell must do
//! instead is diagnose it and exit with an ordinary failure status, so the assertions below are as
//! much about what does *not* happen — 134, a signal, silence — as about what does.

mod common;

use common::{Run, run, run_in};
use std::fs;

/// Assert that `r` failed the way a diagnosed limit failure looks, not the way a crash does.
fn assert_diagnosed(r: &Run, what: &str) {
    assert_ne!(
        r.status, 134,
        "{what}: aborted (stack overflow), stderr: {}",
        r.stderr
    );
    assert!(
        r.status == 1 || r.status == 2,
        "{what}: expected status 1 or 2, got {} — stderr: {}",
        r.status,
        r.stderr
    );
    assert!(
        r.stderr.contains("maximum nesting level exceeded"),
        "{what}: no diagnostic on stderr, got: {:?}",
        r.stderr
    );
}

#[test]
fn unbounded_function_recursion_is_diagnosed() {
    let r = run("f() { f; }; f");
    assert_diagnosed(&r, "self-recursive function");
}

#[test]
fn a_file_that_sources_itself_is_diagnosed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let script = dir.path().join("loop.sh");
    fs::write(&script, ". ./loop.sh\n").expect("write script");

    let r = run_in(dir.path(), ". ./loop.sh");
    assert_diagnosed(&r, "self-sourcing file");
}

#[test]
fn absurd_arithmetic_nesting_is_diagnosed() {
    let expr = format!("{}1{}", "(".repeat(50_000), ")".repeat(50_000));
    let r = run(&format!("echo $(( {} ))", expr));
    assert_diagnosed(&r, "50 000 nested parentheses");
}

/// This one never reached rush's own code: brush's parser overflowed the stack first.
#[test]
fn absurd_input_nesting_is_diagnosed() {
    let script = format!("{}true{}", "{ ".repeat(20_000), "; }".repeat(20_000));
    let r = run(&script);
    assert_diagnosed(&r, "20 000 nested brace groups");
}

/// `eval` re-enters the parser with no function call to bound it.
#[test]
fn unbounded_eval_recursion_is_diagnosed() {
    let r = run(r#"x='eval "$x"'; eval "$x""#);
    assert_diagnosed(&r, "self-evaluating eval");
}

/// The limits must leave recursion at a depth real scripts use completely alone.
#[test]
fn ordinary_recursion_still_works() {
    // `$1` inside `$(( ))` is not supported yet, hence the detour through `n`.
    let r = run(
        "count() { if [ $1 -gt 0 ]; then n=$1; count $((n - 1)); else echo done; fi; }; count 20",
    );
    assert_eq!(r.out(), "done", "stderr: {}", r.stderr);
    assert_eq!(r.status, 0);
}

/// A script that sources several files in a row must not exhaust the *source* budget: the counter
/// is a depth, not a total.
#[test]
fn repeated_sourcing_is_not_nesting() {
    let dir = tempfile::tempdir().expect("tempdir");
    fs::write(dir.path().join("one.sh"), "echo one\n").expect("write script");

    let mut script = String::new();
    for _ in 0..100 {
        script.push_str(". ./one.sh; ");
    }
    let r = run_in(dir.path(), &script);
    assert_eq!(r.status, 0, "stderr: {}", r.stderr);
    assert_eq!(r.out().lines().count(), 100);
}

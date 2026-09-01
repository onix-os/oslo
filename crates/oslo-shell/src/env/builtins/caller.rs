//! `caller` — where the currently executing shell function was called from.
//!
//! The idiom it serves is a stack trace out of an `ERR` or `EXIT` trap:
//! `i=0; while caller $i; do i=$((i+1)); done`. That loop terminates on the exit status, not on
//! the output, which is why the status matters more here than the text: 1 as soon as the frame
//! asked for does not exist, 0 while it does.
//!
//! # What oslo can and cannot report
//!
//! Three fields: line, function, source. oslo tracks the function names
//! ([`Environment::call_stack`]) but has no `LINENO` — no construct in the AST carries a source
//! position — and no per-frame source file. The missing fields are reported as `0` and as
//! bash's own placeholder for a frame it cannot name, `NULL` (`bash -c 'f() { caller; }; f'`
//! prints `1 NULL`), rather than being invented. A fabricated line number would be worse than an
//! obviously absent one: it would send a reader to the wrong line of the right file.

use crate::env::scope::{Environment, UNNAMED_FUNCTION};
use oslo_base::error::Result;

/// The line field, which oslo cannot fill in. Zero, not a plausible-looking number: a made-up
/// line sends a reader to the wrong place with no hint that it did.
const UNKNOWN_LINE: &str = "0";

/// The source field. bash prints this same placeholder whenever the frame came from no file,
/// which under `-c` is every frame.
const UNKNOWN_SOURCE: &str = UNNAMED_FUNCTION;

/// `caller [expr]`.
pub fn builtin_caller(env: &mut Environment, args: &[String]) -> Result<i32> {
    let frames = env.call_stack();
    if frames.is_empty() {
        // Not an error worth a message: bash is silent here, and the `while caller $i` loop
        // depends on it being silent.
        return Ok(1);
    }

    let Some(operand) = args.get(1) else {
        // With no argument bash prints the *caller's* line and source, and nothing else.
        println!("{} {}", UNKNOWN_LINE, UNKNOWN_SOURCE);
        return Ok(0);
    };

    let Ok(index) = operand.parse::<usize>() else {
        crate::env::complain(
            args,
            operand,
            &format!("caller: {operand}: invalid frame specifier"),
            "not a frame number",
            Some("a frame is a non-negative number; 0 is the caller of the function you are in"),
        );
        return Ok(1);
    };
    // `caller n` names the function that *made* the call `n` levels out, not the function the
    // frame belongs to: `f() { g; }; g() { caller 0; }` reports `f`. So frame 0 is the second
    // entry from the top, and the form fails as soon as that entry would be the top-level
    // script — which is why `f() { caller 0; }; f` at the top level exits 1 in bash while a
    // bare `caller` in the same place exits 0.
    let Some(name) = index
        .checked_add(2)
        .and_then(|back| frames.len().checked_sub(back))
        .map(|pos| frames[pos].clone())
    else {
        return Ok(1);
    };
    println!("{} {} {}", UNKNOWN_LINE, name, UNKNOWN_SOURCE);
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::builtin_caller;
    use crate::env::Environment;

    fn argv(words: &[&str]) -> Vec<String> {
        words.iter().map(|s| s.to_string()).collect()
    }

    /// Outside a function there is nothing to report, and the status is what says so — the
    /// `while caller $i` idiom would never terminate otherwise.
    #[test]
    fn outside_a_function_it_fails_quietly() {
        let mut env = Environment::new();
        assert_eq!(builtin_caller(&mut env, &argv(&["caller"])).unwrap(), 1);
        assert_eq!(
            builtin_caller(&mut env, &argv(&["caller", "0"])).unwrap(),
            1
        );
    }

    /// A bare `caller` answers for any active call; the indexed form answers only while there is
    /// a *calling function* to name, which is one frame fewer. Checked against bash both ways.
    #[test]
    fn a_frame_is_reported_for_each_active_call() {
        let mut env = Environment::new();
        env.enter_function_named("outer").unwrap();

        // One frame deep: something is executing, but nothing called it except the script.
        assert_eq!(builtin_caller(&mut env, &argv(&["caller"])).unwrap(), 0);
        assert_eq!(
            builtin_caller(&mut env, &argv(&["caller", "0"])).unwrap(),
            1
        );

        env.enter_function_named("inner").unwrap();
        assert_eq!(
            builtin_caller(&mut env, &argv(&["caller", "0"])).unwrap(),
            0
        );
        assert_eq!(
            builtin_caller(&mut env, &argv(&["caller", "1"])).unwrap(),
            1
        );

        env.exit_function();
        env.exit_function();
        assert_eq!(builtin_caller(&mut env, &argv(&["caller"])).unwrap(), 1);
    }

    #[test]
    fn a_non_numeric_frame_is_refused() {
        let mut env = Environment::new();
        env.enter_function_named("f").unwrap();
        assert_eq!(
            builtin_caller(&mut env, &argv(&["caller", "x"])).unwrap(),
            1
        );
        env.exit_function();
    }
}

//! The one error type the evaluator unwinds with, and how it becomes an exit status.
//!
//! Four of the variants are not failures at all: `exit`, `return`, `break` and `continue` travel
//! as errors only because that is how a value escapes an arbitrarily deep evaluation. Anything
//! that turns an error into a number has to tell the two kinds apart, which is what
//! [`ShellError::control_flow_status`] is for — collapsing them all to 1 is exactly the bug
//! behind `( exit 3 )` reporting 1.

use thiserror::Error;

#[derive(Error, Debug)]
pub enum ShellError {
    #[error("Syntax error: {0}")]
    SyntaxError(String),

    #[error("Expansion error: {0}")]
    ExpansionError(String),

    #[error("Execution error: {0}")]
    ExecutionError(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Lua error: {0}")]
    Lua(#[from] mlua::Error),

    #[error("POSIX error: {0}")]
    Nix(#[from] nix::Error),

    #[error("Builtin exit requested with code: {0}")]
    Exit(i32),

    #[error("Return called with code: {0}")]
    Return(i32),

    #[error("Break called with depth: {0}")]
    Break(usize),

    #[error("Continue called with depth: {0}")]
    Continue(usize),
}

impl ShellError {
    /// The status this error *carries*, for the variants that are control flow rather than
    /// failure — `None` for a genuine error.
    ///
    /// `exit n` and `return n` carry `n`. `break`/`continue` carry nothing: when one escapes the
    /// last enclosing loop there is no loop left to complain to, and every POSIX shell treats it
    /// as a silent no-op with the status of whatever ran before it, so 0 here.
    pub fn control_flow_status(&self) -> Option<i32> {
        match self {
            ShellError::Exit(code) | ShellError::Return(code) => Some(*code),
            ShellError::Break(_) | ShellError::Continue(_) => Some(0),
            _ => None,
        }
    }

    /// The status a genuine failure reports where it is caught.
    ///
    /// 2 for a syntax error — the number POSIX reserves for one and the number `make` and CI
    /// scripts test for — and 1 for everything else. A *fatal* expansion error aborting a
    /// non-interactive shell is a different question, answered by `main`: bash exits 127 there,
    /// but the same error inside a subshell or a pipeline stage still reports 1.
    pub fn failure_status(&self) -> i32 {
        match self {
            ShellError::SyntaxError(_) => 2,
            _ => 1,
        }
    }

    /// The status a fatal error gives a *non-interactive* shell that had to abandon its script.
    ///
    /// An expansion that cannot be performed at all — `${v:?msg}`, a malformed `${...}`, division
    /// by zero, a syntax error in the body of a `$( )` — ends the shell with 127 rather than the
    /// 1 the same failure reports from inside a subshell. That is what bash does and what scripts
    /// testing "the shell gave up" look for. A syntax error counts here because the script itself
    /// parsed before it ran: the only way to raise one *during* execution is a `$( )` body, which
    /// bash also parses late and also reports as 127.
    ///
    /// The documented exception is `${!name}` through a value that is not a name, which bash
    /// unwinds by a different path and exits 1 for. Recognising it by its message is a stopgap —
    /// the honest fix is a distinct variant raised where `expand::param` handles indirection —
    /// but that message is built in exactly one place, and the alternative is knowingly reporting
    /// the wrong number.
    pub fn fatal_exit_status(&self) -> i32 {
        match self {
            ShellError::ExpansionError(msg) if msg.ends_with(INVALID_NAME_SUFFIX) => 1,
            ShellError::SyntaxError(_) | ShellError::ExpansionError(_) => 127,
            other => other.failure_status(),
        }
    }
}

/// Tail of the diagnostic `expand::param` raises for `${!name}` when `name`'s value is not a
/// name. Lives here because [`ShellError::fatal_exit_status`] is its only reader.
pub const INVALID_NAME_SUFFIX: &str = ": invalid variable name";

pub type Result<T> = std::result::Result<T, ShellError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_and_return_carry_their_code() {
        assert_eq!(ShellError::Exit(3).control_flow_status(), Some(3));
        assert_eq!(ShellError::Exit(255).control_flow_status(), Some(255));
        assert_eq!(ShellError::Return(2).control_flow_status(), Some(2));
    }

    #[test]
    fn loop_control_is_a_silent_zero() {
        assert_eq!(ShellError::Break(1).control_flow_status(), Some(0));
        assert_eq!(ShellError::Continue(4).control_flow_status(), Some(0));
    }

    #[test]
    fn real_errors_carry_no_status() {
        assert_eq!(
            ShellError::ExpansionError("x".into()).control_flow_status(),
            None
        );
        assert_eq!(ShellError::SyntaxError("x".into()).failure_status(), 2);
        assert_eq!(ShellError::ExpansionError("x".into()).failure_status(), 1);
        assert_eq!(ShellError::ExecutionError("x".into()).failure_status(), 1);
    }

    #[test]
    fn a_fatal_expansion_gives_up_with_127() {
        // The same error is worth 1 where it is only a failed command, and 127 where it ends the
        // shell — the two must not be conflated.
        let err = ShellError::ExpansionError("v: is unset".into());
        assert_eq!(err.failure_status(), 1);
        assert_eq!(err.fatal_exit_status(), 127);
        assert_eq!(
            ShellError::SyntaxError("in a $( ) body".into()).fatal_exit_status(),
            127
        );
    }

    #[test]
    fn an_invalid_indirect_name_gives_up_with_1() {
        let err = ShellError::ExpansionError(format!("not a name{}", INVALID_NAME_SUFFIX));
        assert_eq!(err.fatal_exit_status(), 1);
    }

    #[test]
    fn a_non_expansion_failure_keeps_its_own_status() {
        assert_eq!(
            ShellError::ExecutionError("x".into()).fatal_exit_status(),
            1
        );
    }
}

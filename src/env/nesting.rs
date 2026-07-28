//! Bounded recursion counters for constructs that re-enter the interpreter.
//!
//! A shell function calling itself, a file sourcing itself and `eval` evaluating text that calls
//! `eval` all recurse through the whole evaluator. Rust turns the resulting stack overflow into
//! `SIGABRT`, so before these counters existed `f() { f; }; f` died with status 134 and a core
//! dump — no diagnostic, no chance for the shell to react. A counter cannot make the recursion
//! safe, but it can stop it while there is still stack left to report the error on.

use crate::error::{Result, ShellError};

/// Deepest shell-function nesting, in the spirit of bash's `FUNCNEST`.
///
/// Measured, not guessed: a debug build overflows its 8 MiB stack somewhere between 300 and 400
/// levels of plain function recursion, and a nested *program* burns the same stack at the same
/// time, so the limits here and in [`crate::parser::nesting`] have to fit in one budget together.
/// 100 is an order of magnitude more than recursive shell code actually uses.
pub const MAX_FUNCTION_DEPTH: usize = 100;

/// Deepest nesting of `source` and `eval`, which re-enter the parser as well as the evaluator.
///
/// Shared by both because they are the same hazard: a file that sources itself and
/// `x='eval "$x"'; eval "$x"` recurse identically.
pub const MAX_SCRIPT_DEPTH: usize = 50;

/// A depth counter that refuses to go past its limit.
///
/// Every entry point is paired: `enter` on the way in, `exit` on the way out *whatever* the
/// outcome, so an unwinding `return`, `break` or error cannot leave the count drifting upwards
/// and eventually refuse a call that is nowhere near the limit.
pub struct DepthGuard {
    depth: usize,
    limit: usize,
}

impl DepthGuard {
    pub const fn new(limit: usize) -> Self {
        Self { depth: 0, limit }
    }

    /// Descend one level, or fail if that would exceed the limit.
    pub fn enter(&mut self) -> Result<()> {
        if self.depth >= self.limit {
            return Err(ShellError::ExecutionError(
                "maximum nesting level exceeded".to_string(),
            ));
        }
        self.depth += 1;
        Ok(())
    }

    /// Come back up one level. Saturating: an unpaired `exit` must not wrap to `usize::MAX`.
    pub fn exit(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    pub fn depth(&self) -> usize {
        self.depth
    }
}

#[cfg(test)]
mod tests {
    use super::DepthGuard;

    #[test]
    fn enters_up_to_the_limit_then_refuses() {
        let mut g = DepthGuard::new(3);
        for _ in 0..3 {
            assert!(g.enter().is_ok());
        }
        let err = g.enter().expect_err("the fourth level is over the limit");
        assert!(
            err.to_string().contains("maximum nesting level exceeded"),
            "{err}"
        );
        // A refused entry must not have counted: unwinding still calls `exit` only for the
        // levels that were actually entered.
        assert_eq!(g.depth(), 3);
    }

    #[test]
    fn unwinding_restores_the_budget() {
        let mut g = DepthGuard::new(2);
        for _ in 0..8 {
            assert!(g.enter().is_ok());
            assert!(g.enter().is_ok());
            assert!(g.enter().is_err());
            g.exit();
            g.exit();
        }
        assert_eq!(g.depth(), 0);
    }

    #[test]
    fn exit_without_enter_stays_at_zero() {
        let mut g = DepthGuard::new(1);
        g.exit();
        assert_eq!(g.depth(), 0);
        assert!(g.enter().is_ok());
    }
}

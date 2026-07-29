//! Turning a SIGINT into an unwind, and back into a status at the top (R7.2).
//!
//! A keystroke has to abort *everything* the shell is in the middle of — nested loops, functions,
//! `if` branches — and then stop being an abort. Those are two different jobs, done in two places:
//!
//! * [`raise`] produces the error that unwinds. It reuses [`ShellError::Exit`] because every
//!   construct in the evaluator already knows to let that one through untouched, which a new
//!   variant would have to be taught in a dozen `match` arms.
//! * [`ListFrame`] recognises the outermost command list *in this process* and converts the
//!   unwind back into a plain status there.
//!
//! The conversion is what keeps a REPL alive. `Err(Exit(130))` escaping to `main` means "the
//! shell is over", which is right for `exit 130` and wrong for Ctrl-C at the prompt. Absorbing it
//! at depth 0 gives both callers what they want without either of them knowing: a script's `main`
//! exits with the 130 it is handed, and the REPL prints its next prompt.
//!
//! [`INTERRUPTING`] is what tells the two apart — a genuine `exit 130` never sets it — and it is
//! a thread-local because it belongs to one evaluation in flight, not to the process. A forked
//! subshell starts at depth 1 or more, so an interrupt inside it stays an unwind and becomes the
//! child's exit status, which is what a subshell is supposed to report.

use crate::env::Environment;
use crate::error::{Result, ShellError};
use std::cell::Cell;

/// The status a shell reports for a command SIGINT ended: 128 + SIGINT.
const INTERRUPTED_STATUS: i32 = 130;

thread_local! {
    /// How many command lists this thread is inside.
    static LIST_DEPTH: Cell<u32> = const { Cell::new(0) };
    /// Whether the error currently unwinding is an interrupt rather than an `exit`.
    static INTERRUPTING: Cell<bool> = const { Cell::new(false) };
}

/// Begin the unwind for a SIGINT that arrived at a command boundary.
///
/// `$?` is set here rather than at the top, because the constructs the error passes through are
/// entitled to read it — an `EXIT` trap fired by a script that was interrupted should see 130.
pub(crate) fn raise(env: &mut Environment) -> ShellError {
    env.last_status = INTERRUPTED_STATUS;
    INTERRUPTING.with(|flag| flag.set(true));
    ShellError::Exit(INTERRUPTED_STATUS)
}

/// One nesting level of [`super::eval_command_list`], and the place an interrupt stops unwinding.
pub(crate) struct ListFrame {
    outermost: bool,
}

impl ListFrame {
    pub(crate) fn enter() -> Self {
        let depth = LIST_DEPTH.with(|d| {
            let entered = d.get();
            d.set(entered + 1);
            entered
        });
        Self {
            outermost: depth == 0,
        }
    }

    /// Let `result` through, unless it is an interrupt that has now run out of shell to unwind.
    ///
    /// Consumes the frame so the depth is restored before the caller sees the value, and so a
    /// second `absorb` on the same frame cannot swallow a real `exit`.
    pub(crate) fn absorb(self, result: Result<i32>) -> Result<i32> {
        let outermost = self.outermost;
        drop(self);

        if !outermost {
            return result;
        }
        // Clear the flag whatever happened: an interrupt that raced the end of an evaluation must
        // not turn the *next* `exit` into a swallowed one.
        let interrupting = INTERRUPTING.with(|flag| flag.replace(false));
        match result {
            Err(ShellError::Exit(status)) if interrupting => Ok(status),
            other => other,
        }
    }
}

impl Drop for ListFrame {
    fn drop(&mut self) {
        LIST_DEPTH.with(|d| d.set(d.get().saturating_sub(1)));
    }
}

#[cfg(test)]
mod tests {
    use super::{INTERRUPTED_STATUS, ListFrame, raise};
    use crate::env::Environment;
    use crate::error::ShellError;
    use crate::exec::job;
    use crate::exec::pipeline::eval_command_list;
    use crate::parser::parse_bash_script;
    use nix::libc;
    use nix::sys::wait::{WaitStatus, waitpid};

    fn run(src: &str) -> (i32, Environment) {
        let mut env = Environment::new();
        let list = parse_bash_script(src).expect("parse");
        let status = eval_command_list(&mut env, &list).expect("interrupt was not absorbed");
        (status, env)
    }

    /// A pending interrupt aborts the evaluation and reports 130 in both `$?` and the status,
    /// rather than being noticed and dropped.
    #[test]
    fn a_pending_interrupt_becomes_status_130() {
        let _ = job::interrupt_pending();
        job::note_interrupt();
        let (status, env) = run("echo unreachable");
        assert_eq!(status, INTERRUPTED_STATUS);
        assert_eq!(env.last_status, INTERRUPTED_STATUS);
    }

    /// R7.2, the finding itself: a loop that never enters the kernel must still be interruptible
    /// *after it has started*. A poll that only ran once, before the loop, would leave this
    /// spinning forever — which is exactly what the shell did.
    ///
    /// Run in a forked child, and for two reasons: the loop is genuinely infinite until the
    /// signal arrives, so a failure has to be observed as a child that never exits rather than as
    /// a wedged test binary; and the child is single-threaded, so an interval timer can deliver
    /// the interrupt to the same thread that is evaluating.
    #[test]
    fn an_interrupt_ends_a_loop_that_has_already_started() {
        let child = unsafe { nix::unistd::fork() }.expect("fork");
        match child {
            nix::unistd::ForkResult::Child => {
                let status = interrupt_a_running_loop();
                // `_exit`, not `exit`: libtest's atexit hooks belong to the parent.
                unsafe { libc::_exit(status) };
            }
            nix::unistd::ForkResult::Parent { child } => {
                assert_eq!(
                    waitpid(child, None).expect("waitpid"),
                    WaitStatus::Exited(child, INTERRUPTED_STATUS),
                    "the loop did not end at 130"
                );
            }
        }
    }

    /// Arrange for an interrupt 100ms from now, then spin until it lands.
    extern "C" fn interrupt_now(_: libc::c_int) {
        job::note_interrupt();
    }

    fn interrupt_a_running_loop() -> i32 {
        unsafe {
            let mut action: libc::sigaction = std::mem::zeroed();
            action.sa_sigaction = interrupt_now as *const () as usize;
            libc::sigaction(libc::SIGALRM, &action, std::ptr::null_mut());

            let timer = libc::itimerval {
                it_interval: libc::timeval {
                    tv_sec: 0,
                    tv_usec: 0,
                },
                it_value: libc::timeval {
                    tv_sec: 0,
                    tv_usec: 100_000,
                },
            };
            libc::setitimer(libc::ITIMER_REAL, &timer, std::ptr::null_mut());
        }

        let mut env = Environment::new();
        let list = parse_bash_script("while true; do :; done; echo unreachable").expect("parse");
        // Any unwind reaching here is a failure of the absorb, reported as a status the parent
        // will not mistake for success.
        eval_command_list(&mut env, &list).unwrap_or(99)
    }

    /// The interrupt unwinds *everything*, not one loop level: nested loops and a function call
    /// in between all come apart.
    #[test]
    fn an_interrupt_unwinds_every_enclosing_construct() {
        let _ = job::interrupt_pending();
        let mut env = Environment::new();
        let list = parse_bash_script("f() { :; }; for i in 1 2 3; do f; done").expect("parse");
        let outer = ListFrame::enter();
        job::note_interrupt();
        let inner_result = eval_command_list(&mut env, &list);
        // The nested evaluation kept unwinding rather than absorbing, because a frame was already
        // open above it — which is what carries the interrupt out of a function and its caller.
        assert!(matches!(inner_result, Err(ShellError::Exit(130))));
        assert!(matches!(outer.absorb(inner_result), Ok(130)));
    }

    /// The flag is what separates the two meanings of `Exit(130)`. A real `exit 130` must still
    /// reach `main` as an error, or an interactive `exit` would stop exiting.
    #[test]
    fn a_genuine_exit_is_not_absorbed() {
        let _ = job::interrupt_pending();
        let mut env = Environment::new();
        let list = parse_bash_script("exit 130").expect("parse");
        assert!(matches!(
            eval_command_list(&mut env, &list),
            Err(ShellError::Exit(130))
        ));
    }

    /// Only the outermost frame absorbs: a nested list has to keep unwinding, or the interrupt
    /// would stop at the first `{ ...; }` it met.
    #[test]
    fn a_nested_frame_lets_the_interrupt_through() {
        let mut env = Environment::new();
        let outer = ListFrame::enter();
        let inner = ListFrame::enter();
        let err = raise(&mut env);
        assert!(matches!(inner.absorb(Err(err)), Err(ShellError::Exit(130))));
        assert!(matches!(
            outer.absorb(Err(ShellError::Exit(INTERRUPTED_STATUS))),
            Ok(INTERRUPTED_STATUS)
        ));
    }
}

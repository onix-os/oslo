//! Process and signal control: `trap`, `wait`, `umask`, `kill`.
//!
//! `kill` and `umask` each grew a real parser — signal specs in one, symbolic modes in the
//! other — and neither has anything to say to the other, so they live in their own files with
//! the signal name table between them.
//!
//! `trap` is split the same way, but along a different seam: [`trap`] is the operand grammar a
//! user types, and [`handlers`] is what the kernel and the evaluator do about it afterwards. The
//! two halves have almost no vocabulary in common — one deals in `SIGINT` and `-p`, the other in
//! `sigaction` and an atomic pending set — and mixing them is how the trap table ended up with no
//! reader at all.

mod handlers;
mod kill;
mod trap;
mod umask;

/// Shared with `trap` and anything else that has to turn a signal name into a number. It is
/// `pub(crate)` so that wiring stays a one-word change in `builtins/mod.rs` rather than a second
/// copy of the table.
pub(crate) mod signals;

pub use handlers::{run_exit_trap, run_pending_traps};
pub use kill::builtin_kill;
pub use trap::builtin_trap;
pub use umask::builtin_umask;

use crate::env::scope::Environment;
use crate::error::Result;

pub fn builtin_wait(env: &mut Environment, args: &[String]) -> Result<i32> {
    use nix::sys::wait::waitpid;
    use nix::unistd::Pid;
    if args.len() < 2 {
        if let Some(bg_pid) = env.last_bg_pid {
            let pid = Pid::from_raw(bg_pid as i32);
            let _ = waitpid(pid, None);
        }
        return Ok(0);
    }

    for arg in &args[1..] {
        if let Ok(pid_i32) = arg.parse::<i32>() {
            let pid = Pid::from_raw(pid_i32);
            let _ = waitpid(pid, None);
        }
    }
    Ok(0)
}

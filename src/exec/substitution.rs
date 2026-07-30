//! Command substitution: `$(cmd)` and backticks.
//!
//! Runs the command in a forked child with stdout on a pipe and captures what it writes.

use crate::env::Environment;
use crate::env::builtins::run_exit_trap;
use crate::error::{Result, ShellError};
use crate::exec::compound::flush_stdout;
use crate::exec::pipeline::{eval_command_list, status_of, wait_for_status};
use nix::unistd::{ForkResult, close, dup2, fork, pipe};
use std::os::fd::{AsRawFd, IntoRawFd};

pub fn eval_command_substitution(env: &mut Environment, cmd_str: &str) -> Result<String> {
    // Parse before forking. In the child the only channel back to the caller is an exit status,
    // so a parse failure there could not be reported as one — it used to `unwrap`, printing a
    // Rust panic on stderr while the parent went on to exit 0.
    let ast =
        crate::parser::parse_with_aliases(cmd_str, &|n| env.get_alias(n).map(str::to_string))?;

    let (reader, writer) =
        pipe().map_err(|e| ShellError::ExecutionError(format!("Pipe failed: {}", e)))?;

    // Anything already buffered belongs to the parent's stdout, not to the captured output.
    flush_stdout();

    unsafe {
        match fork() {
            Ok(ForkResult::Child) => {
                // R4.7: the substitution body is a subshell like any other and must not inherit
                // the shell's own signal policy.
                crate::exec::job::reset_signals_for_child();
                let _ = close(reader.into_raw_fd());
                let _ = dup2(writer.as_raw_fd(), 1);
                let _ = close(writer.into_raw_fd());

                // The child keeps the inherited environment, so `$(helper_fn)` can still see
                // the shell's functions and `$(echo $1)` its positional parameters.
                env.enter_subshell();
                let res = status_of(eval_command_list(env, &ast));
                // Without this the capture pipe would close on an unflushed partial line.
                flush_stdout();
                // The substitution is a shell of its own, so an EXIT trap set inside it runs here
                // — and its output is captured along with everything else the child wrote, which
                // is what bash does too.
                std::process::exit(run_exit_trap(env, res));
            }
            Ok(ForkResult::Parent { child }) => {
                let _ = close(writer.into_raw_fd());
                let mut output = Vec::new();
                use std::io::Read;
                let mut file = std::fs::File::from(reader);
                let _ = file.read_to_end(&mut output);
                // Kept, not discarded: an assignment-only command reports the status of the last
                // substitution in it (`x=$(exit 5)` leaves `$?` at 5), and this is the only place
                // that number exists. `Environment::take_substitution_status` consumes it.
                env.note_substitution_status(wait_for_status(child));
                // A shell word is a C string, so it cannot carry a NUL. bash drops NUL bytes
                // from substitution output rather than truncating or aborting; keeping them
                // would push a NUL into argv and kill the `CString` conversion at exec time.
                // The warning is bash's too: silently losing bytes is worth telling someone.
                let len_with_nuls = output.len();
                output.retain(|&b| b != 0);
                if output.len() != len_with_nuls {
                    eprintln!("oslo: warning: command substitution: ignored null byte in input");
                }
                Ok(String::from_utf8_lossy(&output).into_owned())
            }
            Err(e) => Err(ShellError::ExecutionError(format!("Fork failed: {}", e))),
        }
    }
}

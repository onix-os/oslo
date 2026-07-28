//! Command substitution: `$(cmd)` and backticks.
//!
//! Runs the command in a forked child with stdout on a pipe and captures what it writes.

use crate::env::Environment;
use crate::error::{Result, ShellError};
use crate::exec::pipeline::eval_command_list;
use nix::sys::wait::waitpid;
use nix::unistd::{ForkResult, close, dup2, fork, pipe};
use std::os::fd::{AsRawFd, IntoRawFd};

pub fn eval_command_substitution(env: &mut Environment, cmd_str: &str) -> Result<String> {
    // Parse before forking. In the child the only channel back to the caller is an exit status,
    // so a parse failure there could not be reported as one — it used to `unwrap`, printing a
    // Rust panic on stderr while the parent went on to exit 0.
    let ast = crate::parser::parse_bash_script(cmd_str)?;

    let (reader, writer) =
        pipe().map_err(|e| ShellError::ExecutionError(format!("Pipe failed: {}", e)))?;
    let vars = env.get_all_vars();

    unsafe {
        match fork() {
            Ok(ForkResult::Child) => {
                let _ = close(reader.into_raw_fd());
                let _ = dup2(writer.as_raw_fd(), 1);
                let _ = close(writer.into_raw_fd());

                let mut child_env = Environment::new();
                for (k, v) in vars {
                    child_env.set_var(&k, &v, true);
                }
                let res = eval_command_list(&mut child_env, &ast).unwrap_or(1);
                std::process::exit(res);
            }
            Ok(ForkResult::Parent { child }) => {
                let _ = close(writer.into_raw_fd());
                let mut output = Vec::new();
                use std::io::Read;
                let mut file = std::fs::File::from(reader);
                let _ = file.read_to_end(&mut output);
                let _ = waitpid(child, None);
                // A shell word is a C string, so it cannot carry a NUL. bash drops NUL bytes
                // from substitution output rather than truncating or aborting; keeping them
                // would push a NUL into argv and kill the `CString` conversion at exec time.
                // The warning is bash's too: silently losing bytes is worth telling someone.
                let len_with_nuls = output.len();
                output.retain(|&b| b != 0);
                if output.len() != len_with_nuls {
                    eprintln!("rush: warning: command substitution: ignored null byte in input");
                }
                Ok(String::from_utf8_lossy(&output).into_owned())
            }
            Err(e) => Err(ShellError::ExecutionError(format!("Fork failed: {}", e))),
        }
    }
}

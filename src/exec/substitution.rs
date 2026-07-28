//! Command substitution: `$(cmd)` and backticks.
//!
//! Runs the command in a forked child with stdout on a pipe and captures what it writes.

use crate::env::Environment;
use crate::error::{Result, ShellError};
use crate::exec::pipeline::eval_command_list;
use crate::lexer::Lexer;
use crate::parser::Parser;
use nix::sys::wait::waitpid;
use nix::unistd::{ForkResult, close, dup2, fork, pipe};
use std::os::fd::{AsRawFd, IntoRawFd};

pub fn eval_command_substitution(env: &mut Environment, cmd_str: &str) -> Result<String> {
    let (reader, writer) =
        pipe().map_err(|e| ShellError::ExecutionError(format!("Pipe failed: {}", e)))?;
    let vars = env.get_all_vars();

    unsafe {
        match fork() {
            Ok(ForkResult::Child) => {
                let _ = close(reader.into_raw_fd());
                let _ = dup2(writer.as_raw_fd(), 1);
                let _ = close(writer.into_raw_fd());

                let ast =
                    if let Ok(parsed) = crate::parser::brush_adapter::parse_bash_script(cmd_str) {
                        parsed
                    } else {
                        let lexer = Lexer::new(cmd_str);
                        let mut parser = Parser::new(lexer);
                        parser.parse_command_list().unwrap()
                    };

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
                Ok(String::from_utf8_lossy(&output).to_string())
            }
            Err(e) => Err(ShellError::ExecutionError(format!("Fork failed: {}", e))),
        }
    }
}

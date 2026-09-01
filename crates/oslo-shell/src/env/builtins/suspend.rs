//! `suspend` — stop this shell until someone continues it.
//!
//! Only meaningful under job control: the shell stops itself and its process group with
//! `SIGSTOP`, and whatever started it (a parent shell, a terminal multiplexer) is left to notice
//! and resume it with `SIGCONT`. Without job control there is nothing on the other side to do the
//! resuming, so bash refuses rather than wedging the session, and so does this.

use crate::env::origin_now;
use crate::env::scope::Environment;
use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;
use oslo_base::error::Result;

/// `suspend [-f]`.
pub fn builtin_suspend(env: &mut Environment, args: &[String]) -> Result<i32> {
    let mut force = false;
    for arg in &args[1..] {
        match arg.as_str() {
            "-f" => force = true,
            "--" => break,
            other => {
                crate::env::complain_with_usage(
                    args,
                    other,
                    &format!("suspend: {other}: invalid option"),
                    "not an option here",
                    "suspend: usage: suspend [-f]",
                );
                return Ok(2);
            }
        }
    }

    if !env.monitor() {
        // bash's own wording, and its status. A shell run from a script has no job control, so
        // this is the answer `bash -c suspend` gives too.
        eprintln!("{}suspend: cannot suspend: no job control", origin_now());
        return Ok(1);
    }
    if is_login_shell(env) && !force {
        eprintln!("{}suspend: cannot suspend a login shell", origin_now());
        return Ok(1);
    }

    // The whole process group, not just this process: a shell that stopped alone would leave its
    // children running while nothing was reading their input. This is what bash does.
    match kill(Pid::from_raw(0), Signal::SIGSTOP) {
        Ok(()) => Ok(0),
        Err(errno) => {
            eprintln!("{}suspend: cannot suspend: {}", origin_now(), errno);
            Ok(1)
        }
    }
}

/// Whether this is a login shell, which by convention is spelled with a leading `-` in `$0`.
fn is_login_shell(env: &Environment) -> bool {
    env.shell_name.starts_with('-')
}

#[cfg(test)]
mod tests {
    use super::builtin_suspend;
    use crate::env::Environment;
    use crate::env::options::ShellOption;

    fn argv(words: &[&str]) -> Vec<String> {
        words.iter().map(|s| s.to_string()).collect()
    }

    /// Without job control there is nobody to resume the shell, so stopping it would hang the
    /// session with no way out. Refused, exactly as bash refuses it.
    #[test]
    fn without_job_control_it_refuses() {
        let mut env = Environment::new();
        assert_eq!(builtin_suspend(&mut env, &argv(&["suspend"])).unwrap(), 1);
    }

    /// A login shell is the one nothing else is waiting on; `-f` is the way to insist.
    #[test]
    fn a_login_shell_is_refused_until_forced() {
        let mut env = Environment::new();
        env.set_option(ShellOption::Monitor, true);
        env.shell_name = "-oslo".to_string();
        assert_eq!(builtin_suspend(&mut env, &argv(&["suspend"])).unwrap(), 1);
    }

    #[test]
    fn an_unknown_option_is_a_usage_error() {
        let mut env = Environment::new();
        assert_eq!(
            builtin_suspend(&mut env, &argv(&["suspend", "-z"])).unwrap(),
            2
        );
    }
}

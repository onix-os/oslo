//! `oslo secret run VAR=NAME … -- CMD…` — one value, one child, and nowhere else.
//!
//! # Why this exists beside `$(oslo secret get …)`
//!
//! A command substitution puts the value in the *calling shell*, where it is a variable that every
//! later command inherits, that a `set` prints, and that a crash dump or a `/proc/PID/environ` read
//! can find for as long as that shell lives. This runs one program with one extra variable in its
//! environment and no shell in between, so the value's lifetime is the child's.
//!
//! It is still the child's environment, which is readable by the user who started it — that is what
//! an environment is. What it buys is that the value is not in anything else.

use oslo::secrets::Store;

use super::fail;
use super::help::MENU;

pub fn run(store: &Store, args: &[String]) -> i32 {
    let Some(at) = args.iter().position(|arg| arg == "--") else {
        return MENU.wrong("run", "needs a `--` before the command to run");
    };
    let (wanted, command) = args.split_at(at);
    let command = &command[1..];
    if wanted.is_empty() || command.is_empty() {
        return MENU.wrong("run", "needs both a VAR=NAME and a command");
    }

    let mut environment = Vec::with_capacity(wanted.len());
    for pair in wanted {
        let Some((variable, name)) = pair.split_once('=') else {
            return fail(&format!("{pair:?} is not VAR=NAME"));
        };
        if variable.is_empty() {
            return fail(&format!("{pair:?} has no variable to set"));
        }
        // The name defaults to the variable, lowercased and hyphenated the way a store name usually
        // reads: `oslo secret run GITHUB_TOKEN= -- gh` means the secret `github-token`.
        let name = match name.is_empty() {
            true => variable.to_lowercase().replace('_', "-"),
            false => name.to_string(),
        };
        match store.get(&name) {
            Ok(value) => match String::from_utf8(value) {
                Ok(value) => environment.push((variable.to_string(), value)),
                Err(_) => return fail(&format!("{name}: not text, so it cannot be a variable")),
            },
            Err(e) => return fail(&e),
        }
    }

    let error = exec(command, &environment);
    fail(&format!("{}: {error}", command[0]));
    127
}

/// Replace this process with the command, so nothing is left holding the value.
fn exec(command: &[String], environment: &[(String, String)]) -> std::io::Error {
    use std::os::unix::process::CommandExt;
    std::process::Command::new(&command[0])
        .args(&command[1..])
        .envs(environment.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .exec()
}

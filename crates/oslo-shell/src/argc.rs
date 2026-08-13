//! `argc` — a script's own arguments, parsed from the comments that declare them.
//!
//! ```sh
//! #!/usr/bin/env oslo
//! # @describe   Send it somewhere
//! # @option -t --tries <NUM>   how many times
//! # @arg     target!           where to
//! argc "$@"                    # ← and now $argc_tries and $argc_target are set
//! ```
//!
//! # Why this is a builtin and not the `eval` line
//!
//! In bash the same feature is spelled `eval "$(argc --argc-eval "$0" "$@")"`, which is a program to
//! find, a fork, a pipe, a quoting round trip and an `eval` of whatever came back. oslo has the
//! parser linked in, so the builtin runs it and assigns the results **directly**:
//!
//! ```text
//! bash:  argc(1) ──► text ──► eval ──► variables
//! oslo:  argc     ──────────────────► variables
//! ```
//!
//! What that buys, beyond the fork: errors are reported the way every other builtin reports them,
//! and it works for a script that has **no file**. A macro-stored script runs from memory with `$0`
//! set to its own name, so `--argc-eval "$0"` has no path to read — the idiom bash needs cannot work
//! there, and this needs no path at all.
//!
//! `oslo --argc-eval` exists too, for bash scripts; see `src/cli.rs`. Both are the same parse.

mod call;
pub mod complete;
mod runtime;

use crate::env::Environment;
use argc::ArgcValue;

pub use runtime::Shell;

/// `argc "$@"` — parse this script's arguments and apply them.
///
/// The status is the shell's: `0` when the parse succeeded, and whatever `argc` asked for when it
/// did not — `0` for `--help`, which printed what was wanted, and `1` for a real mistake.
pub fn builtin_argc(env: &mut Environment, args: &[String]) -> oslo_base::error::Result<i32> {
    // `$0` is the script's name, which is what a stored macro has instead of a path and what a file
    // on disk has as well as one. The runtime tries the macro store first and the filesystem after.
    let name = env.shell_name.clone();
    let source = match source_of(env, &name) {
        Some(source) => source,
        // **Nothing to parse means this was not called from a script**, which at a prompt is what
        // `argc` on its own is: `$0` is the shell. Every other builtin answers `--help` with what it
        // is for, and reporting "cannot read the script" about the shell binary explains nothing.
        None => {
            let asked = args
                .iter()
                .skip(1)
                .any(|word| word == "--help" || word == "-h");
            let usage = self_help(&name);
            if asked {
                println!("{usage}");
                return Ok(0);
            }
            eprintln!("{usage}");
            return Ok(2);
        }
    };

    // The words `argc` matches: the script's name, then the arguments as given. **`args[0]` is the
    // builtin's own name** — every builtin here is called with it, the way `argv[0]` works — so it
    // is dropped: what `argc` wants after the name is what the script was called with.
    //
    // The *base* name, because this word is what the generated help calls the command. `$0` for a
    // script found on `$PATH` is the path it was found at, and `USAGE: /usr/local/bin/deploy` names
    // something nobody types. The path is still passed separately, which is what reads the file.
    let mut words = vec![basename(&name)];
    words.extend(args.iter().skip(1).cloned());

    let runtime = Shell::new(env);
    let values = match argc::eval(runtime, &source, &words, Some(&name), width()) {
        Ok(values) => values,
        Err(problem) => {
            eprintln!("oslo: argc: {problem}");
            return Ok(1);
        }
    };
    apply(env, &values)
}

/// Set what the parse decided, and answer with the status the script carries on with.
///
/// **`--help` and a usage error end the script**, as `ShellError::Exit`. That is not a convenience:
/// the bash rendering ends its text with `exit 0` or `exit 1`, so a script that asked for help stops
/// there rather than running its body with nothing set. A builtin that merely returned a status
/// would leave the next line to run — observed, before this returned the error instead.
fn apply(env: &mut Environment, values: &[ArgcValue]) -> oslo_base::error::Result<i32> {
    let mut call_this: Option<String> = None;
    for value in values {
        match value {
            ArgcValue::Single(name, value) => {
                env.set_var(&argc_name(name), value, false);
            }
            ArgcValue::SingleFn(name, function) => {
                let out = crate::exec::substitution::eval_command_substitution(env, function)
                    .unwrap_or_default();
                env.set_var(&argc_name(name), &out, false);
            }
            ArgcValue::Multiple(name, values) => set_array(env, &argc_name(name), values),
            ArgcValue::PositionalSingle(name, value) => {
                env.set_var(&argc_name(name), value, false);
            }
            ArgcValue::PositionalSingleFn(name, function) => {
                let out = crate::exec::substitution::eval_command_substitution(env, function)
                    .unwrap_or_default();
                env.set_var(&argc_name(name), &out, false);
            }
            ArgcValue::PositionalMultiple(name, values) => set_array(env, &argc_name(name), values),
            ArgcValue::ExtraPositionalMultiple(values) => {
                set_array(env, "argc__positionals", values);
            }
            ArgcValue::Env(name, value) | ArgcValue::EnvFn(name, value) => {
                env.set_var(name, value, true);
            }
            ArgcValue::Map(name, map) => {
                // A `@option --set <K=V>` collected as pairs. Written flat, one array of `k=v`:
                // the shell has no associative array a script could read back portably.
                let flat: Vec<String> = map
                    .iter()
                    .map(|(key, values)| format!("{key}={}", values.join(",")))
                    .collect();
                set_array(env, &argc_name(name), &flat);
            }
            ArgcValue::CommandFn(function) => call_this = Some(function.clone()),
            ArgcValue::ParamFn(_) | ArgcValue::Hook(_) => {}
            ArgcValue::Dotenv(_) | ArgcValue::RequireTools(_) => {}
            ArgcValue::ExternalSubcommand(..) => {}
            // **Printed and done.** `--help` and a usage error both arrive here; the status is the
            // one `argc` chose, and `0` for help is what makes `script --help` succeed.
            ArgcValue::Error((message, status)) => {
                if *status == 0 {
                    println!("{message}");
                } else {
                    eprintln!("{message}");
                }
                return Err(oslo_base::error::ShellError::Exit(*status));
            }
        }
    }

    // The subcommand the arguments selected. Called here so a script is `argc "$@"` and nothing
    // else — the same thing the `eval` line does by leaving a call at the end of what it prints.
    match call_this {
        Some(function) => match crate::syntax::parse_bash_script(&function) {
            Ok(program) => crate::exec::eval_command_list(env, &program),
            Err(problem) => {
                eprintln!("oslo: argc: {problem}");
                Ok(1)
            }
        },
        None => Ok(0),
    }
}

/// What `argc` is, for somebody who typed it at a prompt.
///
/// The name is included because `$0` is the only reason this is being printed: it says which script
/// was looked for and did not exist, which is the useful half of the old error message.
fn self_help(name: &str) -> String {
    format!(
        "usage: argc [ARG]...        — from inside a script, as `argc \"$@\"`\n\
         \n\
         Parses the script's own `# @option`, `# @flag`, `# @arg` and `# @cmd` comments and sets\n\
         `$argc_*` from them, generating `--help` and reporting a bad argument.\n\
         \n\
         There is no script here: `$0` is {name}. In a bash script the same thing is spelled\n\
         `eval \"$(oslo --argc-eval \"$0\" \"$@\")\"`."
    )
}

/// The last component of a path, which is what a command is called.
fn basename(name: &str) -> String {
    name.rsplit('/').next().unwrap_or(name).to_string()
}

/// `argc_tries`, from `tries`. The prefix is argc's, and scripts are written against it.
fn argc_name(id: &str) -> String {
    format!("argc_{}", id.replace('-', "_"))
}

fn set_array(env: &mut Environment, name: &str, values: &[String]) {
    // An indexed array, which is what `argc_files=( a b )` is in the bash rendering.
    let mut array = crate::env::scope::ShellArray::default();
    for (at, value) in values.iter().enumerate() {
        array.set(at as i64, value.clone());
    }
    env.set_array(name, array);
}

/// The script's source, by name.
fn source_of(env: &mut Environment, name: &str) -> Option<String> {
    use argc::Runtime;
    Shell::new(env).read_to_string(name)
}

/// How wide help text may be, from the terminal.
fn width() -> Option<usize> {
    oslo_ui::dropdown::width::terminal_cols().checked_sub(2)
}

#[cfg(test)]
#[path = "argc/tests.rs"]
mod tests;

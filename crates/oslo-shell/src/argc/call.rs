//! Running a script's own function to answer a question `argc` asked.
//!
//! A declaration can name a function instead of a literal:
//!
//! ```text
//! # @option --dir=`_default_dir`     the default is whatever this prints
//! # @arg host[`_choice_host`]        the choices are whatever this prints, one per line
//! ```
//!
//! Upstream answers by exec'ing `bash script.sh ___internal___ _default_dir …` and reading stdout.
//! Here the script's source is already in hand and the shell that wants the answer *is* a shell, so
//! the source is sourced and the function called inside a command substitution — one fork, no bash,
//! and the same capture rule `$(…)` uses.
//!
//! # It cannot change the shell that called it
//!
//! Command substitution runs in a subshell, so a `# @option --dir=`_d`` whose function exports a
//! variable or changes directory affects nothing here. That is the same guarantee the bash version
//! gives by starting a separate process, arrived at more cheaply.

use crate::env::Environment;
use std::collections::HashMap;

/// Call `function` with `args` against `source`, and answer with what it printed.
///
/// Empty on any failure. **A question that cannot be answered is a default that is not there**, and
/// `argc` treats an empty answer as exactly that — so a broken helper costs the caller a default
/// rather than the whole parse.
pub(super) fn capture(
    env: &mut Environment,
    source: &str,
    function: &str,
    args: &[String],
    envs: &HashMap<String, String>,
) -> String {
    let mut script = String::new();
    // The variables `argc` wants the function to see — `ARGC_OS`, and whatever a choice function is
    // told about the arguments so far. Written as assignments in front of the call so they are the
    // subshell's and go away with it.
    for (name, value) in envs {
        if is_a_name(name) {
            script.push_str(&format!("{name}={}\n", quoted(value)));
        }
    }
    // **The source is sourced, not run.** A script that ends in `argc "$@"` would otherwise parse
    // its arguments again, from inside the answer to a question its own parse asked.
    script.push_str("__argc_no_run=1\n");
    script.push_str(source);
    script.push('\n');
    script.push_str(function);
    for arg in args {
        script.push(' ');
        script.push_str(&quoted(arg));
    }
    script.push('\n');

    // **Trailing newlines come off**, as they do from `$(…)` and as upstream's bash runtime trims
    // them: a choice function ends with `echo`, and the empty last line it leaves would otherwise
    // be offered as a choice.
    crate::exec::substitution::eval_command_substitution(env, &script)
        .unwrap_or_default()
        .trim_end_matches('\n')
        .to_string()
}

/// Whether a word may be written as a shell variable name.
fn is_a_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// One word, quoted so the shell reads it back as itself.
fn quoted(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **Quoting only, no subshell.** What a helper function actually *does* is tested through the
    /// real binary in `tests/argc_tests.rs`: command substitution forks, and forking from a test
    /// process with a dozen other threads in it is how a suite hangs — which this one did, once.
    #[test]
    fn a_value_is_written_so_the_shell_reads_it_back_as_itself() {
        assert_eq!(quoted("plain"), "'plain'");
        assert_eq!(quoted("with space"), "'with space'");
        assert_eq!(
            quoted("$HOME"),
            "'$HOME'",
            "no expansion inside single quotes"
        );
        assert_eq!(
            quoted("it's"),
            "'it'\\''s'",
            "the one character that ends it"
        );
        assert_eq!(quoted("*"), "'*'");
    }

    #[test]
    fn only_a_name_becomes_an_assignment() {
        assert!(is_a_name("ARGC_OS"));
        assert!(is_a_name("_x1"));
        assert!(!is_a_name("1BAD"));
        assert!(!is_a_name("has-dash"));
        assert!(!is_a_name(""));
    }
}

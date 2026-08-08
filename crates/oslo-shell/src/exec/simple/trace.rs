//! `set -x`: the execution trace.
//!
//! One line per simple command, on stderr, after expansion and before the command runs. The
//! "after expansion" half is the whole value of the option: a trace of the *source* text would
//! show `cp $src $dst`, which is exactly the two words whose values the reader is trying to find
//! out. oslo prints what the command is actually about to receive.
//!
//! stderr, never stdout: the trace has to stay out of `$(…)` and out of a redirected `> out`, or
//! turning the option on changes the script's own output.

use crate::env::Environment;

/// Emit the trace line for a command, if `set -x` is on.
///
/// `assignments` are the `NAME=value` prefixes in the order they were written, already expanded;
/// `words` is the expanded argument vector. Either may be empty — `x=1` on its own is traced with
/// no words, and a plain `echo hi` with no assignments.
pub fn trace_command(env: &Environment, assignments: &[(String, String)], words: &[String]) {
    if !env.xtrace() {
        return;
    }
    let fields = assignments
        .iter()
        .map(|(name, value)| format!("{}={}", name, quote_for_trace(value)))
        .chain(words.iter().map(|word| quote_for_trace(word)));
    eprintln!("{}{}", ps4(env), fields.collect::<Vec<_>>().join(" "));
}

/// The trace prefix: `$PS4`, or `+ ` when it is unset.
///
/// Taken literally rather than re-expanded. bash expands PS4 on every trace line, which is how
/// `PS4='+ $LINENO '` works; oslo has no `LINENO` yet, and expanding here would mean running
/// command substitutions from inside the tracer — with the tracer's own `set -x` still on.
fn ps4(env: &Environment) -> String {
    env.get_var("PS4").unwrap_or("+ ").to_string()
}

/// Quote a word the way the trace has to, so the line says how many arguments there were.
///
/// An empty argument and an argument containing a space are both invisible in a bare
/// space-joined line — `cmd '' x` and `cmd x` would trace identically — and that ambiguity is
/// precisely what someone reading a trace is trying to resolve.
fn quote_for_trace(word: &str) -> String {
    let plain = !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_alphanumeric() || "_-.,:/=+@%^".contains(c));
    if plain {
        return word.to_owned();
    }
    // Single quotes protect everything but a single quote itself, which is spliced in the only
    // way the shell can: close, escape, reopen.
    format!("'{}'", word.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::{ps4, quote_for_trace};
    use crate::env::Environment;

    #[test]
    fn ps4_defaults_to_a_plus_and_a_space() {
        let mut env = Environment::new();
        assert_eq!(ps4(&env), "+ ");
        env.set_var("PS4", "## ", false);
        assert_eq!(ps4(&env), "## ");
    }

    #[test]
    fn a_word_is_quoted_only_when_the_line_would_otherwise_lie() {
        assert_eq!(quote_for_trace("echo"), "echo");
        assert_eq!(quote_for_trace("/usr/bin/env"), "/usr/bin/env");
        assert_eq!(quote_for_trace(""), "''");
        assert_eq!(quote_for_trace("a b"), "'a b'");
        assert_eq!(quote_for_trace("it's"), r"'it'\''s'");
    }
}

//! The macros a spec file names that only a shell can answer.
//!
//! ```yaml
//! completion:
//!   positional:
//!     - ["$(git branch --format '%(refname:short)')"]
//!     - ["$bash(compgen -A hostname)"]
//! ```
//!
//! # `$(…)` does not fork a shell, because oslo is one
//!
//! carapace has to run `sh -c` here: it is a completion binary, and the shell it is completing for
//! is somewhere else. oslo is the shell, so the command goes through the same command-substitution
//! path `$(…)` on a real line goes through — one fork, no `sh` on `$PATH` required, and the same
//! capture rule. It is the trick `argc::call` already uses, for the same reason.
//!
//! A shell that is named — `$bash(…)`, `$zsh(…)` — is run as itself, because a spec that asks for
//! bash is asking for bash's own completions. One that is not installed answers nothing rather than
//! an error: a spec is written for many machines and a missing shell is a fact about this one.
//!
//! # What the command is told
//!
//! The variables of the line — `C_VALUE`, `C_ARG0…`, `C_FLAG_…` — as assignments, and the words
//! already typed as `"$@"`. That is what carapace passes and it is what a `compgen` line needs.

use oslo_ui::spec::action::{Offer, Query};

/// Shells that may be named, and that oslo will not pretend to be.
const SHELLS: &[&str] = &[
    "bash", "zsh", "fish", "nu", "elvish", "xonsh", "osh", "pwsh", "cmd",
];

/// Answer one macro, or nothing at all.
pub fn offers(name: &str, arg: &str, query: &Query) -> Vec<Offer> {
    match name {
        // `$(cmd)` and `$sh(cmd)` are oslo's own.
        "" | "sh" => rows(&here(arg, query)),
        shell if SHELLS.contains(&shell) => rows(&elsewhere(shell, arg, query)),
        // `$spec(file)` hands the rest of the parse to another spec file, which is a re-entry into
        // the walk rather than a list of values. Not read yet, and quiet rather than wrong.
        _ => Vec::new(),
    }
}

/// Run `command` in this shell, in a subshell, and answer with what it printed.
fn here(command: &str, query: &Query) -> String {
    let mut script = preamble(query);
    if !query.dir.is_empty() {
        script.push_str(&format!(
            "cd {} 2>/dev/null || exit 0\n",
            quoted(&query.dir)
        ));
    }
    script.push_str(command);
    script.push('\n');

    let mut env = crate::env::Environment::new();
    crate::exec::substitution::eval_command_substitution(&mut env, &script).unwrap_or_default()
}

/// Run `command` in the shell it named.
fn elsewhere(shell: &str, command: &str, query: &Query) -> String {
    let Ok(program) = which::which(shell) else {
        return String::new();
    };
    let mut process = std::process::Command::new(program);
    match shell {
        "cmd" => {
            process.arg("/c").arg(command);
        }
        "nu" | "pwsh" | "elvish" | "xonsh" => {
            process.arg("-c").arg(command);
        }
        // The POSIX-ish ones take the words after a `--`, so the command sees them as `"$@"`.
        _ => {
            process.arg("-c").arg(command).arg("--");
            process.args(query.words.iter().skip(1));
        }
    }
    for (name, value) in variables(query) {
        process.env(name, value);
    }
    if !query.dir.is_empty() {
        process.current_dir(&query.dir);
    }
    process
        .output()
        .ok()
        .map(|out| String::from_utf8_lossy(&out.stdout).into_owned())
        .unwrap_or_default()
}

/// The assignments and the `set --` a command is run under.
fn preamble(query: &Query) -> String {
    let mut script = String::new();
    for (name, value) in variables(query) {
        script.push_str(&format!("{name}={}\n", quoted(&value)));
    }
    // The words the command was typed with, minus the command's own name — the same `"$@"`
    // carapace hands a `sh -c … -- "$@"`.
    script.push_str("set --");
    for word in query.words.iter().skip(1) {
        script.push(' ');
        script.push_str(&quoted(word));
    }
    script.push('\n');
    script
}

/// `C_VALUE`, `C_ARG0…`, `C_FLAG_…`: what the line has said so far.
fn variables(query: &Query) -> Vec<(String, String)> {
    let mut out = vec![("C_VALUE".to_string(), query.value.clone())];
    for (index, arg) in query.args.iter().enumerate() {
        out.push((format!("C_ARG{index}"), arg.clone()));
    }
    for (name, value) in &query.flags {
        if is_a_name(name) {
            out.push((format!("C_FLAG_{name}"), value.clone()));
        }
    }
    out
}

/// What a command printed, one offer per line.
fn rows(output: &str) -> Vec<Offer> {
    output
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.is_empty())
        .map(|line| {
            let mut fields = line.splitn(3, '\t');
            Offer {
                value: fields.next().unwrap_or_default().to_string(),
                description: fields.next().filter(|d| !d.is_empty()).map(str::to_string),
                tag: None,
            }
        })
        .collect()
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

    /// **Quoting and shaping only, no subshell.** Command substitution forks, and forking from a
    /// test process with a dozen threads in it is how a suite hangs — the same rule `argc::call`
    /// follows, and for the same reason it was learnt.
    #[test]
    fn the_line_reaches_the_command_as_variables_and_arguments() {
        let mut flags = std::collections::HashMap::new();
        flags.insert("FILE".to_string(), "out.txt".to_string());
        let query = Query {
            args: vec!["build".into()],
            words: vec!["deploy".into(), "build".into()],
            value: "part".into(),
            flags,
            dir: String::new(),
        };
        let script = preamble(&query);
        assert!(script.contains("C_VALUE='part'"), "{script}");
        assert!(script.contains("C_ARG0='build'"), "{script}");
        assert!(script.contains("C_FLAG_FILE='out.txt'"), "{script}");
        assert!(script.ends_with("set -- 'build'\n"), "{script}");
    }

    #[test]
    fn a_word_is_written_so_the_shell_reads_it_back_as_itself() {
        assert_eq!(quoted("it's"), "'it'\\''s'");
        assert_eq!(quoted("$HOME"), "'$HOME'");
    }

    #[test]
    fn every_printed_line_is_an_offer_and_a_tab_splits_off_its_description() {
        let offers = rows("one\ntwo\twith description\n\nthree\tstyled\tblue\n");
        assert_eq!(offers.len(), 3);
        assert_eq!(offers[0].value, "one");
        assert_eq!(offers[1].description.as_deref(), Some("with description"));
        // The third field is carapace's style, which oslo paints from its own theme.
        assert_eq!(offers[2].value, "three");
        assert_eq!(offers[2].description.as_deref(), Some("styled"));
    }

    /// A macro naming a shell that is not installed answers nothing. A spec is written for many
    /// machines; a missing shell is a fact about this one, not an error in the spec.
    #[test]
    fn a_shell_that_is_not_here_is_quiet() {
        let query = Query::default();
        assert!(offers("definitely-not-a-shell", "echo hi", &query).is_empty());
        assert!(offers("spec", "other.yaml", &query).is_empty());
    }
}

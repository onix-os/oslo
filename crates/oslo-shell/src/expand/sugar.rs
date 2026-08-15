//! Interactive-only shorthands: `=command` and `@name`.
//!
//! Both are zsh conveniences that a POSIX shell has no business doing to a *script*, and both are
//! gated on the shell being interactive for exactly that reason. `echo =foo` in a script written
//! for `/bin/sh` must print `=foo`, because that is what every other `sh` does and oslo is meant to
//! be one. At a prompt, where you typed the line yourself a moment ago, the shorthand is worth
//! having and there is nobody to surprise.
//!
//! # `=command`
//!
//! `nvim =script` becomes `nvim /usr/bin/script`. The rule is zsh's, including the part that makes
//! it tolerable: **a name that resolves to nothing is left exactly as it was**. So `echo =nosuch`
//! still prints `=nosuch`, and the only words this changes are ones where the change is what you
//! meant.
//!
//! # `@name` lives elsewhere
//!
//! `cd @work` becomes `cd /home/u/data/code/tools`, from the marks file and the table a config
//! registers. A distinct sigil rather than zsh's `~name`, and deliberately: `~work` already means
//! "the home directory of the user called `work`", so overloading it means a real user account can
//! silently shadow your shortcut — or the reverse, which is worse.
//!
//! **It is substituted where a tilde is** — `expand::word::marked_directory`, before splitting and
//! globbing — because it names a *directory* and everything after it is the user's own path.
//! Applied here, at the end, `@proj/*.rs` reached the command with a literal `*` while `~/*.rs`
//! expanded, and `echo "@proj"` expanded through the quotes because a finished string no longer
//! remembers it had any.
//!
//! `=command` stays here, and the difference is the point: it answers with a *command's* path,
//! which must not then be globbed or split again.

use crate::env::Environment;
use crate::expand::word::{Field, Origin, Run};

pub use oslo_base::dirs::{named_dir, named_dirs, set_named_dirs};

/// Apply `=command` to one already-expanded field.
///
/// `None` when the field is not one — which is every field of almost every command. Answering with
/// the field copied back meant a `String` per argument per interactive command, to say that nothing
/// had happened.
pub fn expand_field(env: &Environment, field: &str) -> Option<String> {
    if !env.interactive() {
        return None;
    }
    equals(field.strip_prefix('=')?)
}

/// `=name` — where that command lives, or `None` if it is not a command.
///
/// `None` rather than an error, and that distinction is the whole safety argument: an unresolved
/// name leaves the word alone rather than failing the command, so the worst case of this feature
/// is that nothing happens.
fn equals(name: &str) -> Option<String> {
    if name.is_empty() || name.contains('/') {
        return None;
    }
    which::which(name)
        .ok()
        .map(|p| p.to_string_lossy().into_owned())
}

/// Replace a leading `@name` with the directory it stands for, as a quoted run.
///
/// `Origin::Quoted` for the substituted half, exactly as [`WordPart::Tilde`] does it: a home
/// directory that happens to contain a `*` is still just a directory, and the same is true of a
/// marked one. Everything after it keeps the origin it had, so the user's own glob still globs.
///
/// Only the *first* run, and only when it is text the script itself wrote: `echo "@proj"` is a
/// literal, and a `@proj` that arrived out of a variable is data rather than a shorthand.
pub(crate) fn marked_directory(field: Field) -> Field {
    let Some(first) = field.first() else {
        return field;
    };
    if first.origin != Origin::Literal || !first.text.starts_with('@') {
        return field;
    }
    let rest = &first.text[1..];
    let cut = rest.find('/').unwrap_or(rest.len());
    let Some(path) = oslo_base::dirs::named_dir(&rest[..cut]) else {
        return field;
    };
    let tail = &rest[cut..];
    let mut out = vec![Run::new(path, Origin::Quoted)];
    if !tail.is_empty() {
        out.push(Run::new(tail, first.origin));
    }
    out.extend(field.into_iter().skip(1));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn interactive() -> Environment {
        let mut env = Environment::new();
        env.set_option(crate::env::options::ShellOption::Interactive, true);
        env
    }

    /// What the field becomes: the rewrite, or the field itself when there was none.
    fn field(env: &Environment, text: &str) -> String {
        expand_field(env, text).unwrap_or_else(|| text.to_string())
    }

    /// The safety property: a script sees none of this, because `echo =foo` has to print `=foo`
    /// the way every other `/bin/sh` does.
    #[test]
    fn a_script_gets_none_of_it() {
        let env = Environment::new();
        assert_eq!(field(&env, "=sh"), "=sh");
        assert_eq!(field(&env, "@work"), "@work");
    }

    /// `=name` resolves to a path, and a name that resolves to nothing is left alone — which is
    /// what keeps the worst case of this feature to "nothing happened".
    #[test]
    fn equals_resolves_a_command_and_ignores_everything_else() {
        let env = interactive();
        let resolved = field(&env, "=sh");
        assert!(resolved.starts_with('/'), "{resolved:?}");
        assert!(resolved.ends_with("/sh"), "{resolved:?}");

        assert_eq!(
            field(&env, "=definitely-not-a-command"),
            "=definitely-not-a-command"
        );
        assert_eq!(field(&env, "="), "=");
        // A path is not a command name; `=./x` is left for the filesystem to answer for.
        assert_eq!(field(&env, "=/bin/sh"), "=/bin/sh");
        // And a word that merely contains `=` is untouched: `FOO=bar` must survive.
        assert_eq!(field(&env, "FOO=bar"), "FOO=bar");
    }

    /// **`@name` is no longer this function's business.** It is substituted where a tilde is —
    /// before splitting and globbing — so that `@proj/*.rs` globs and `echo "@proj"` does not
    /// expand. Handled here it did neither. See `expand::word::marked_directory`.
    #[test]
    fn at_name_is_not_handled_here_any_more() {
        let env = interactive();
        set_named_dirs(HashMap::from([(
            "work".to_string(),
            "/home/u/work".to_string(),
        )]));

        assert_eq!(field(&env, "@work"), "@work");
        assert_eq!(field(&env, "@work/src/main.rs"), "@work/src/main.rs");

        set_named_dirs(HashMap::new());
    }
}

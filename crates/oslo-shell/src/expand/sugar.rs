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
//! # `@name`
//!
//! `cd @work` becomes `cd /home/u/data/code/tools`, from a table a config registers. A distinct
//! sigil rather than zsh's `~name`, and deliberately: `~work` already means "the home directory of
//! the user called `work`", so overloading it means a real user account can silently shadow your
//! shortcut — or the reverse, which is worse.

use crate::env::Environment;
use std::collections::HashMap;
use std::sync::RwLock;

/// The directories `@name` can reach, from `oslo.dirs`.
static NAMED: RwLock<Option<HashMap<String, String>>> = RwLock::new(None);

/// Register the `@name` table. Replaces whatever was there.
///
/// A leading `~` is resolved here rather than at use: a config writes `~/data/code`, and the
/// expansion path should not have to know about tildes to answer what `@work` means.
pub fn set_named_dirs(dirs: HashMap<String, String>) {
    let home = std::env::var("HOME").unwrap_or_default();
    let resolved = dirs
        .into_iter()
        .map(|(name, path)| {
            let path = match path.strip_prefix('~') {
                Some(rest) if !home.is_empty() && (rest.is_empty() || rest.starts_with('/')) => {
                    format!("{home}{rest}")
                }
                _ => path,
            };
            (name, path)
        })
        .collect();
    if let Ok(mut slot) = NAMED.write() {
        *slot = Some(resolved);
    }
}

/// The path `@name` stands for, if one was registered.
pub fn named_dir(name: &str) -> Option<String> {
    NAMED.read().ok()?.as_ref()?.get(name).cloned()
}

/// Apply the interactive shorthands to one already-expanded field.
///
/// `None` when the field is not one of these — which is every field of almost every command.
/// Answering with the field copied back meant a `String` per argument per interactive command, to
/// say that nothing had happened.
pub fn expand_field(env: &Environment, field: &str) -> Option<String> {
    if !env.interactive() {
        return None;
    }
    if let Some(rest) = field.strip_prefix('=') {
        return equals(rest);
    }
    if let Some(rest) = field.strip_prefix('@') {
        return at_name(rest);
    }
    None
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

/// `@name` and `@name/tail` — the registered directory, with anything after the first `/` kept.
fn at_name(rest: &str) -> Option<String> {
    let (name, tail) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, ""),
    };
    if name.is_empty() {
        return None;
    }
    Some(format!("{}{tail}", named_dir(name)?))
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn at_name_expands_a_registered_directory_and_keeps_the_tail() {
        let env = interactive();
        set_named_dirs(HashMap::from([(
            "work".to_string(),
            "/home/u/work".to_string(),
        )]));

        assert_eq!(field(&env, "@work"), "/home/u/work");
        assert_eq!(field(&env, "@work/src/main.rs"), "/home/u/work/src/main.rs");
        // An unregistered name is left alone, as an unresolved `=` is.
        assert_eq!(field(&env, "@nowhere"), "@nowhere");
        assert_eq!(field(&env, "@"), "@");

        set_named_dirs(HashMap::new());
    }
}

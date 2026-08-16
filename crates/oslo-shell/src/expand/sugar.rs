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
//! `nvim =script` becomes `nvim /usr/bin/script`. The rule is zsh's, with one departure: **a name
//! that resolves to nothing is reported, not passed through.** zsh leaves the word as it was, on
//! the argument that the worst case is then "nothing happened". The worst case is not that —
//! `ldd =olso`, one transposed pair, handed `ldd` a word starting with `=` and came back with
//! `ldd: ./=olso: No such file or directory`, which names a file nobody wrote and never mentions
//! the shorthand. See [`equals`].
//!
//! Quoting takes it back: `echo "=ls"` is a literal, exactly as `echo "@proj"` is.
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
//! Applied at the end instead, `@proj/*.rs` reached the command with a literal `*` while `~/*.rs`
//! expanded, and `echo "@proj"` expanded through the quotes because a finished string no longer
//! remembers it had any.
//!
//! **`=command` is applied there too, and for the last of those reasons.** It used to run at the
//! end, on the finished strings, which is why quoting did not protect it either. What it answers
//! with is marked [`Origin::Quoted`], so a command's path is still never split or globbed
//! afterwards — which is what the old placement was buying, kept without the cost.

use crate::env::Environment;
use crate::expand::word::{Field, Origin, Run};

pub use oslo_base::dirs::{named_dir, named_dirs, set_named_dirs};

/// What one field turned out to be.
pub enum Equals {
    /// Not a `=command` at all — every field of almost every command.
    NotSugar,
    /// `=name`, and this is where `name` lives.
    Found(String),
    /// `=name`, and there is no such command. Carries the name, for the message.
    Unknown(String),
}

/// Apply `=command` to one field, before it is split or globbed.
///
/// **Only a field the user typed unquoted, and only one run of it.** `echo "=ls"` is a literal and
/// has to stay one — the same rule `@name` follows, and the same one it had to be taught after
/// `echo "@proj"` was found expanding through its quotes. A field of several runs is not the
/// shorthand either: `=$cmd` is a variable's value, which is data rather than something typed.
///
/// The answer is substituted as [`Origin::Quoted`], exactly as a mark is, so the path it produces
/// is not then split on `$IFS` or read for glob characters. That is what lets this run *before*
/// those steps, which is where the origin is still known.
pub(crate) fn equals_field(env: &Environment, field: Field) -> std::result::Result<Field, String> {
    if !env.interactive() {
        return Ok(field);
    }
    let [only] = field.as_slice() else {
        return Ok(field);
    };
    if only.origin != Origin::Literal {
        return Ok(field);
    }
    let Some(name) = only.text.strip_prefix('=') else {
        return Ok(field);
    };
    match equals(name) {
        Equals::Found(path) => Ok(vec![Run::new(path, Origin::Quoted)]),
        Equals::Unknown(name) => Err(refusal(env, &name)),
        Equals::NotSugar => Ok(field),
    }
}

/// What to say about a name that is not a command — and what it probably was.
///
/// **The near-miss is the whole message.** `olso is not a command` is true and useless: the reason
/// you typed it is that you believe it is one, so being told otherwise leaves you reading your own
/// line for the difference. Two letters the wrong way round is the commonest typo there is, and
/// [`command_index::nearest`] is already built to catch exactly that — the same suggestion the
/// repair offers after a mistyped line, said here at the moment the shorthand fails instead.
fn refusal(env: &Environment, name: &str) -> String {
    let path = env.get_var("PATH").unwrap_or_default();
    match oslo_ui::command_index::nearest(path, name) {
        Some(near) => format!("={name}: {name} is not a command — did you mean ={near}?"),
        None => format!("={name}: {name} is not a command"),
    }
}

/// `=name` — where that command lives, or why it could not be said.
///
/// **An unresolved name is reported rather than passed through**, and that is a deliberate reversal.
/// The rule used to be zsh's, which leaves the word exactly as it was, on the argument that the
/// worst case of the feature is then that nothing happens. The worst case turned out to be worse
/// than that: `ldd =olso` — one transposed pair — handed `ldd` a word beginning with `=`, and what
/// came back was `ldd: ./=olso: No such file or directory`, which blames a file nobody meant to
/// name and says nothing about the shorthand that produced it. A typo you made a moment ago should
/// be told to you, not smuggled into an argument list.
///
/// It is safe here in a way it would not be in a script precisely because this is interactive-only:
/// `echo =foo` in a `/bin/sh` script still prints `=foo`, untouched, because none of this runs.
///
/// The three shapes that are *not* the shorthand stay untouched: `=` alone, `==x`, and anything
/// with a `/` in it, which is already a path.
fn equals(name: &str) -> Equals {
    if name.is_empty() || name.contains('/') || name.starts_with('=') {
        return Equals::NotSugar;
    }
    match which::which(name) {
        Ok(p) => Equals::Found(p.to_string_lossy().into_owned()),
        Err(_) => Equals::Unknown(name.to_string()),
    }
}

/// Replace a leading `@name` with the directory it stands for, as a quoted run.
///
/// `Origin::Quoted` for the substituted half, exactly as [`WordPart::Tilde`] does it: a home
/// directory that happens to contain a `*` is still just a directory, and the same is true of a
/// marked one. Everything after it keeps the origin it had, so the user's own glob still globs.
///
/// Only the *first* run, and only when it is text the script itself wrote: `echo "@proj"` is a
/// literal, and a `@proj` that arrived out of a variable is data rather than a shorthand.
pub(crate) fn marked_directory(field: Field) -> std::result::Result<Field, String> {
    let Some(first) = field.first() else {
        return Ok(field);
    };
    if first.origin != Origin::Literal || !first.text.starts_with('@') {
        return Ok(field);
    }
    let rest = &first.text[1..];
    let cut = rest.find('/').unwrap_or(rest.len());
    let Some(path) = oslo_base::dirs::named_dir(&rest[..cut]) else {
        return match nearest_mark(&rest[..cut]) {
            Some(near) => Err(format!(
                "@{name}: no mark called {name} — did you mean @{near}?",
                name = &rest[..cut]
            )),
            None => Ok(field),
        };
    };
    let tail = &rest[cut..];
    let mut out = vec![Run::new(path, Origin::Quoted)];
    if !tail.is_empty() {
        out.push(Run::new(tail, first.origin));
    }
    out.extend(field.into_iter().skip(1));
    Ok(out)
}

/// The nearest registered mark to `name`, or `None` when nothing is close.
///
/// **`None` is the common answer and the important one.** `@` on its own, `@{u}` and `@~1` are git
/// revisions people type constantly, and none of them is a mistyped mark — so unless a name is
/// *near* one the user actually registered, the word is left exactly as it was. That is what keeps
/// this from breaking `git log @` the way a blanket refusal would.
fn nearest_mark(name: &str) -> Option<String> {
    let marks = oslo_base::dirs::named_dirs();
    oslo_ui::command_index::nearest_of(marks.iter().map(|(name, _)| name.as_str()), name)
}

/// `@name` substituted in every field, for the entry points that do not go through
/// `expand_word_at`.
///
/// **The same substitution, at every door.** It was applied in one place — arguments only — so
/// `[ -d @proj ]` was true while `[[ -d @proj ]]` was false, and `case @proj in /tmp*)` did not
/// match where `case ~ in /tmp*)` did: the same word written two ways, giving opposite answers.
/// This module's own contract says a mark is substituted where a tilde is, and a tilde is a
/// `WordPart` that every one of those paths already resolves.
pub(crate) fn marked_fields(
    env: &Environment,
    fields: Vec<Field>,
) -> std::result::Result<Vec<Field>, String> {
    if !env.interactive() {
        return Ok(fields);
    }
    fields.into_iter().map(marked_directory).collect()
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

    /// What an unquoted field becomes: the rewrite, or the text itself when there was none.
    fn field(env: &Environment, text: &str) -> String {
        typed(env, text, Origin::Literal)
    }

    /// The same, for a field the user wrote inside quotes.
    fn quoted(env: &Environment, text: &str) -> String {
        typed(env, text, Origin::Quoted)
    }

    /// The rewritten text, or `<refused: …>` carrying the message the shell would print.
    fn typed(env: &Environment, text: &str, origin: Origin) -> String {
        match equals_field(env, vec![Run::new(text, origin)]) {
            Ok(runs) => runs.into_iter().map(|r| r.text).collect(),
            Err(message) => format!("<refused: {message}>"),
        }
    }

    /// The safety property: a script sees none of this, because `echo =foo` has to print `=foo`
    /// the way every other `/bin/sh` does.
    #[test]
    fn a_script_gets_none_of_it() {
        let env = Environment::new();
        assert_eq!(field(&env, "=sh"), "=sh");
        assert_eq!(field(&env, "@work"), "@work");
    }

    /// `=name` resolves to a path, and the three shapes that are not the shorthand are left alone.
    #[test]
    fn equals_resolves_a_command_and_ignores_everything_else() {
        let env = interactive();
        let resolved = field(&env, "=sh");
        assert!(resolved.starts_with('/'), "{resolved:?}");
        assert!(resolved.ends_with("/sh"), "{resolved:?}");

        assert_eq!(field(&env, "="), "=");
        // A path is not a command name; `=./x` is left for the filesystem to answer for.
        assert_eq!(field(&env, "=/bin/sh"), "=/bin/sh");
        // And a word that merely contains `=` is untouched: `FOO=bar` must survive.
        assert_eq!(field(&env, "FOO=bar"), "FOO=bar");
        // `==x` is not a name either — the second `=` is part of what was typed.
        assert_eq!(field(&env, "==sh"), "==sh");
    }

    /// **A name that resolves to nothing is reported, not passed through.**
    ///
    /// This is the case a person actually meets, and it used to be silent: `ldd =olso` — one
    /// transposed pair — handed `ldd` the literal word `=olso`, and the answer came back as
    /// `ldd: ./=olso: No such file or directory`, blaming a file nobody meant to name. The
    /// shorthand knows the name is wrong at the moment it fails to resolve it, and that is the
    /// moment to say so.
    #[test]
    fn a_name_that_is_not_a_command_is_reported_rather_than_passed_on() {
        let env = interactive();
        let refused = field(&env, "=definitely-not-a-command");
        assert!(refused.starts_with("<refused:"), "{refused}");
        assert!(
            refused.contains("definitely-not-a-command is not a command"),
            "{refused}"
        );
        // And still nothing at all in a script, which is what makes reporting safe here.
        assert_eq!(
            field(&Environment::new(), "=definitely-not-a-command"),
            "=definitely-not-a-command"
        );
    }

    /// **Quoting protects it, in both directions.**
    ///
    /// `echo "=ls"` is a literal and used to expand anyway — the same bug `@name` was found to have
    /// and was fixed for. It matters more now than it did: without this, a quoted `=typo` would not
    /// merely expand, it would *fail the command*.
    #[test]
    fn a_quoted_word_is_never_the_shorthand() {
        let env = interactive();
        assert_eq!(quoted(&env, "=sh"), "=sh");
        assert_eq!(
            quoted(&env, "=definitely-not-a-command"),
            "=definitely-not-a-command"
        );
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

    /// What `marked_directory` does with a name that is not a mark.
    fn mark(text: &str) -> String {
        match marked_directory(vec![Run::new(text, Origin::Literal)]) {
            Ok(runs) => runs.into_iter().map(|r| r.text).collect(),
            Err(message) => format!("<refused: {message}>"),
        }
    }

    /// **A mistyped mark is named; a word that merely starts with `@` is not.**
    ///
    /// The same failure `=name` had — `cd @wrok` answered `cd: @wrok: No such file or directory`,
    /// blaming a directory nobody wrote. It cannot be fixed the same way, though: `@` on its own,
    /// `@{u}` and `@~1` are git revisions people type all day, and refusing every unknown `@word`
    /// would break `git log @`. So this speaks *only* when the name is near a mark that actually
    /// exists, and stays silent otherwise.
    #[test]
    fn a_mistyped_mark_is_named_but_a_git_revision_is_not() {
        set_named_dirs(HashMap::from([(
            "work".to_string(),
            "/home/u/work".to_string(),
        )]));

        let refused = mark("@wrok");
        assert!(refused.contains("did you mean @work?"), "{refused}");

        // Everything git types, untouched — none of it is near `work`.
        for revision in ["@", "@{u}", "@~1", "@HEAD"] {
            assert_eq!(mark(revision), revision, "{revision} was not left alone");
        }

        set_named_dirs(HashMap::new());
        // And with no marks registered at all, nothing is ever near anything.
        assert_eq!(mark("@wrok"), "@wrok");
    }
}

//! `\command` and `\\command` — asking for the thing the shell's own name is standing in front of.
//!
//! # What a backslash means, and what it did not
//!
//! POSIX gives a leading backslash exactly one job: the command word is quoted, so it is not an
//! alias. bash and dash do that and nothing more, and so did oslo. There was no way at all to say
//! "not the builtin" short of naming a path.
//!
//! That gap stopped being theoretical when `rm` became a builtin. A shell where `rm` moves things
//! to `/tmp` needs a one-keystroke way to get the `rm` that does not, and `command rm` — which
//! bypasses *functions*, not builtins — is not it.
//!
//! | written | alias | function | builtin | runs |
//! |---|---|---|---|---|
//! | `cmd` | expanded | used | used | the shell's |
//! | `\cmd` | skipped | skipped | skipped | the program on `$PATH` |
//! | `\\cmd` | expanded | used | skipped | the alias's target, unbuiltin |
//!
//! `\cmd` reads as "whatever this shell has done to that name, give me the program". `\\cmd` is
//! the narrower one, for when the alias *is* the point — `alias rm='rm -i'` — and the builtin is
//! not.
//!
//! # Only at a prompt
//!
//! **`\cmd` in a script keeps its POSIX meaning exactly**, and that is not caution for its own
//! sake: oslo is meant to be `/bin/sh`, so a script on this machine that escapes a name and then
//! expects the builtin must go on getting it. Changing that would be a silent reinterpretation of
//! every `\ls` and `\echo` already written, with no error to notice.
//!
//! This is the same rule `=command`, `@name` and `rm`'s own extensions follow, and it is the house
//! rule for exactly this reason.
//!
//! `\\cmd` has no POSIX meaning to protect — it is a literal backslash followed by a word, and
//! every shell answers `\cmd: command not found` — so it is additive wherever it lands. It is
//! gated the same way anyway, so that the two forms are one feature rather than two.
//!
//! # Where the backslash survives to
//!
//! Nowhere, by the time the word is a string: the lexer strips it. What it leaves behind is the
//! *shape* of the word, and that is what is read here — [`WordPart::Escaped`] is a separate
//! variant from `Literal` precisely so that escaping stays visible after the character is gone.
//!
//! * `\rm` → `[Escaped("r"), Literal("m")]`
//! * `\\rm` → `[Escaped("\\"), Literal("rm")]`
//! * `r\m` → `[Literal("r"), Escaped("m")]` — not a leading escape, and not this
//! * `"rm"` → `[DoubleQuoted(…)]` — quoting a command name has never skipped a builtin, in any
//!   shell, and must not start now

use oslo_base::ast::{Word, WordPart};

/// What a leading backslash on the command word asks the shell to skip.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) enum Escape {
    /// No leading backslash. Ordinary command search.
    #[default]
    None,
    /// `\cmd`: alias, function and builtin all skipped — the program on `$PATH` wins.
    ToProgram,
    /// `\\cmd`: the alias was expanded and the function still counts; only the builtin is skipped.
    PastBuiltin,
}

impl Escape {
    /// Whether the shell's own `cmd` — its builtin — is out of the running.
    pub(crate) fn skips_builtin(self) -> bool {
        self != Escape::None
    }

    /// Whether a shell function by this name is out of the running too.
    ///
    /// Only `\cmd`. `\\cmd` is the narrow form: it says nothing about functions, and a config that
    /// wraps a command in one is not what a user is escaping past when they kept the alias.
    pub(crate) fn skips_function(self) -> bool {
        self == Escape::ToProgram
    }
}

/// What the command word's shape asks for, and how much of it the lexer left on the name.
///
/// The `usize` is how many leading bytes of the *expanded* word are backslash rather than name:
/// one for `\\cmd`, whose word really is `\cmd` by the time it is a string, and zero for `\cmd`,
/// whose backslash the lexer already consumed.
pub(crate) fn intent(word: &Word, interactive: bool) -> (Escape, usize) {
    // The gate is here rather than at the call site so that there is one place to read the rule,
    // and one place a test can pin it.
    if !interactive {
        return (Escape::None, 0);
    }
    match word.parts.first() {
        Some(WordPart::Escaped(text)) if text == "\\" => {
            // `\\` on its own is the word `\`, not an escape of anything. Without this, `\\` alone
            // would ask for a command whose name is the empty string.
            if word.parts.len() > 1 {
                (Escape::PastBuiltin, 1)
            } else {
                (Escape::None, 0)
            }
        }
        // Any other leading escape is `\c…`, the POSIX alias-suppressing form. The lexer has
        // already eaten the backslash, so nothing needs trimming.
        Some(WordPart::Escaped(text)) if !text.is_empty() => (Escape::ToProgram, 0),
        _ => (Escape::None, 0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::syntax::parse_bash_script;

    /// The command word of the one simple command in `source`.
    fn command_word(source: &str) -> Word {
        let ast = parse_bash_script(source).expect("parses");
        let item = ast.items.first().expect("one item");
        let oslo_base::ast::Command::Simple(simple) =
            item.and_or.first.commands.first().expect("one")
        else {
            panic!("not a simple command");
        };
        simple.words.first().expect("a command word").clone()
    }

    #[test]
    fn the_three_forms_are_told_apart() {
        assert_eq!(intent(&command_word("rm x"), true), (Escape::None, 0));
        assert_eq!(
            intent(&command_word("\\rm x"), true),
            (Escape::ToProgram, 0)
        );
        assert_eq!(
            intent(&command_word("\\\\rm x"), true),
            (Escape::PastBuiltin, 1)
        );
    }

    /// An escape that is not at the front is an ordinary word. `r\m` is the command `rm`, and has
    /// been in every shell for forty years.
    #[test]
    fn an_escape_inside_the_word_asks_for_nothing() {
        assert_eq!(intent(&command_word("r\\m x"), true), (Escape::None, 0));
    }

    /// Quoting a command name has never skipped a builtin. `"echo"` and `'echo'` run the builtin
    /// in bash, dash and zsh, and a shell that changed that would break `"$cmd"` dispatch tables.
    #[test]
    fn quoting_the_name_is_not_escaping_it() {
        assert_eq!(intent(&command_word("\"rm\" x"), true), (Escape::None, 0));
        assert_eq!(intent(&command_word("'rm' x"), true), (Escape::None, 0));
    }

    /// A script sees none of this: `\cmd` there means what POSIX says it means.
    #[test]
    fn a_script_gets_the_posix_reading() {
        assert_eq!(intent(&command_word("\\rm x"), false), (Escape::None, 0));
        assert_eq!(intent(&command_word("\\\\rm x"), false), (Escape::None, 0));
    }

    /// `\\` alone is a word, not a prefix, and must not be read as escaping the empty name.
    #[test]
    fn a_lone_double_backslash_is_a_word() {
        assert_eq!(intent(&command_word("\\\\"), true), (Escape::None, 0));
    }
}

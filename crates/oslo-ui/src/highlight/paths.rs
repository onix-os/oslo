//! What the highlighter is willing to say about a word that looks like a path.
//!
//! Two questions, and both of them are about *not* answering when the shell knows something the
//! lexer's spans do not: a word carrying a `$VAR` cannot be resolved here at all, and a word
//! carrying a brace list is not the literal text it appears to be.

use super::lex::{Role, Span};

/// Whether a parameter names something that is actually there.
pub(super) fn names_an_existing_file(word: &str) -> bool {
    if word.is_empty() || word.starts_with('-') {
        return false;
    }
    // `~` and `@name` are not expanded by the lexer, so they are expanded here — a path typed with
    // either is exactly the kind that does exist and would otherwise never light up.
    //
    // All four tilde forms, through the shell's own expander: knowing only `~` and `~/…` left
    // `~root` and `~+/src` reading as paths that are not there, though the shell resolves both.
    let expanded = match word.starts_with('~') {
        true => oslo_base::tilde::expand_prefix(word, &oslo_base::tilde::from_process),
        false => match word.strip_prefix('@') {
            Some(rest) => match oslo_base::dirs::expand_at(rest) {
                Some(path) => path,
                None => return false,
            },
            None => word.to_string(),
        },
    };
    // **A brace list is answered by what it expands to.** `three/{four,five}` is one word to the
    // lexer — `{`, `}` and `,` are not glob metacharacters — so the literal text was handed to the
    // filesystem, no file has ever been called that, and a word naming two real paths was painted
    // exactly like one naming none. Tab has understood braces since it was written; this is the
    // view that had not caught up.
    //
    // Every branch has to exist, not merely one: `{four,nope}` names something that is not there,
    // and saying otherwise would be the same wrong answer in the other direction.
    let alternatives = oslo_base::brace::expand_braces_text(&expanded);
    if alternatives.len() > 1 {
        return alternatives
            .iter()
            .all(|one| std::fs::symlink_metadata(one).is_ok());
    }
    std::fs::symlink_metadata(&expanded).is_ok()
}

/// Which `Word` spans are part of a word that also contains a `$VAR`.
///
/// The lexer splits `$PWD/tmp` into a `Variable` span and a `Word` span, and the existence check is
/// asked of the spans one at a time — so it was asked whether `/tmp` exists, from the filesystem
/// root. `ls $PWD/tmp` came back underlined as a path that is not there while `ls $PWD/one` rendered
/// plain, both of them exactly backwards.
///
/// The highlighter cannot resolve a variable: it has no environment, and the value can be anything.
/// So it says nothing rather than something wrong — the same rule `glob_word_answers` follows for a
/// quoted piece, which it also refuses to have an opinion about.
pub(super) fn touches_a_variable(spans: &[Span]) -> Vec<bool> {
    let joined = |role: Role| {
        matches!(
            role,
            Role::Word
                | Role::Glob
                | Role::Number
                | Role::SingleQuote
                | Role::DoubleQuote
                | Role::Variable
                | Role::Escape
        )
    };
    let mut flagged = vec![false; spans.len()];
    let mut at = 0;
    while at < spans.len() {
        if !joined(spans[at].role) {
            at += 1;
            continue;
        }
        let start = at;
        while at < spans.len() && joined(spans[at].role) {
            at += 1;
        }
        if spans[start..at].iter().any(|s| s.role == Role::Variable) {
            for flag in flagged.iter_mut().take(at).skip(start) {
                *flag = true;
            }
        }
    }
    flagged
}

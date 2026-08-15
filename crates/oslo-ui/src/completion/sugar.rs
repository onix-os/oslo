//! Completing oslo's own interactive shorthands, `=name` and `@name`.
//!
//! Both are rewritten during expansion by `oslo_shell::expand::sugar`, and neither looks like what
//! it becomes. `=grep` names a command and `@work` names a registered directory, so completing
//! either as an ordinary path answers a question nobody asked.

use super::{CompletionCandidate, matches_prefix};
use crate::OsloHelper;
use crate::words::{Quote, Word, quote_replacement};

/// Retarget `=name` at the command name inside it.
///
/// The tail is what `=` resolves through `$PATH`, so it completes as a command rather than a file.
/// A tail containing `/` is not a command name — the shorthand declines those too — and is left for
/// the ordinary path completion.
pub(super) fn equals_segment(word: Word<'_>) -> Word<'_> {
    if word.quote != Quote::None {
        return word;
    }
    let Some(name) = word.text.strip_prefix('=') else {
        return word;
    };
    if name.contains('/') {
        return word;
    }
    Word {
        start: word.start + 1,
        text: name,
        stem: name.to_string(),
        command_position: true,
        carried: 0,
        ..word
    }
}

/// Retarget `\cmd` and `\\cmd` at the name after the escape.
///
/// **The escape is the whole point of the word.** `\rm` asks for the program on `$PATH` rather than
/// oslo's trash-moving builtin, and completion replaced the word entire — so choosing `rm` from the
/// menu deleted the backslash and quietly turned the line back into the builtin it was written to
/// avoid, with nothing on screen saying so.
///
/// Only in command position, which is the only place the escape means anything.
pub(super) fn escaped_segment(word: Word<'_>) -> Word<'_> {
    if word.quote != Quote::None || !word.command_position {
        return word;
    }
    // `\\cmd` is the second spelling, and `unquote` has already collapsed it to `\cmd` in the stem —
    // so the raw text is what says which was typed.
    let escape = if word.text.starts_with("\\\\") {
        2
    } else if word.text.starts_with('\\') {
        1
    } else {
        return word;
    };
    let name = &word.text[escape..];
    if name.is_empty() || name.contains('/') {
        return word;
    }
    Word {
        start: word.start + escape,
        text: name,
        stem: name.to_string(),
        carried: 0,
        ..word
    }
}

/// Retarget `@name/tail` at the path under the directory `@name` stands for.
///
/// Left alone when the word is not that shape, or when the name was never registered — an
/// unregistered `@nowhere` expands to itself, so completing it against the filesystem is the honest
/// answer.
///
/// Only the resolved directory is carried, so the completion writes back the `/src/main.rs` the
/// user is building and leaves the `@work` they typed alone.
pub(super) fn at_segment(word: Word<'_>) -> Word<'_> {
    if word.quote != Quote::None {
        return word;
    }
    let Some(rest) = word.text.strip_prefix('@') else {
        return word;
    };
    let Some(slash) = rest.find('/') else {
        return word;
    };
    let Some(resolved) = oslo_base::dirs::named_dir(&rest[..slash]) else {
        return word;
    };
    let tail = &word.text[1 + slash..];
    Word {
        start: word.start + 1 + slash,
        stem: format!("{resolved}{}", crate::words::unquote(tail)),
        text: tail,
        carried: resolved.len(),
        command_position: false,
        ..word
    }
}

impl OsloHelper {
    /// The registered `@name` directories, for a word that is still just `@` and a partial name.
    pub(super) fn named_dir_candidates(&self, stem: &str, out: &mut Vec<CompletionCandidate>) {
        let Some(prefix) = stem.strip_prefix('@') else {
            return;
        };
        for (name, path) in oslo_base::dirs::named_dirs() {
            if !matches_prefix(&name, prefix, self.case_sensitive()) {
                continue;
            }
            let display = format!("@{name}/");
            out.push(CompletionCandidate {
                replacement: quote_replacement(&display, Quote::None),
                display,
                description: None,
                kind: Some("dir".to_string()),
                path: Some(path),
                detail: None,
            });
        }
    }
}

#[cfg(test)]
mod tests;

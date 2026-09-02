//! What the scalar verbs share: finding the strings they were given.
//!
//! `text` and `path` are the same shape of tool — one column in, one column out, and the strings
//! can arrive three different ways. That arrival is the only part identical enough to be written
//! once; the subcommands and their flags have nothing in common and stay apart.

use super::super::value::{Record, Val};

/// A refusal, and the word the caret belongs under.
///
/// **The word is the whole point.** `crate::env::complain` draws a caret only when it is given one
/// of the command's own words to point at, so an error that carries only a sentence can never be
/// more than a line — and `text` and `path` were built that way, which made their failures the odd
/// ones out among the builtins. See `docs/features/diagnostics.md`.
pub struct Wrong {
    /// The word at fault, when one of the command's words is.
    pub word: Option<String>,
    /// The one-liner, exactly as it goes to a pipe.
    pub message: String,
    /// What goes against the caret — a few words about *this* word.
    ///
    /// **Carried with the error rather than supplied where it is reported.** The reporting site
    /// knows only that something failed, so a label chosen there has to cover every failure it
    /// might be: `path sort --key size` drew "not an option here" under `size`, which is not an
    /// option and was never claimed to be. The site that decided the word is the only one that can
    /// say what is wrong with it.
    pub label: String,
}

impl Wrong {
    /// A refusal with a word to point at, and what to say against it.
    pub fn at(word: &str, label: &str, message: impl Into<String>) -> Wrong {
        Wrong {
            word: Some(word.to_string()),
            message: message.into(),
            label: label.to_string(),
        }
    }

    /// A refusal with nothing to point at, which stays a line on a terminal too. A missing file is
    /// the shape: there is nothing wrong with the *word*, so a box around it is decoration.
    pub fn plain(message: impl Into<String>) -> Wrong {
        Wrong {
            word: None,
            message: message.into(),
            label: String::new(),
        }
    }
}

/// Report a refusal, with the usage block as its help.
///
/// `verb` is what goes in front of the message — `text split`, `path is` — so the one-liner reads
/// the way it always has and the transport face does not move.
pub fn refuse(words: &[String], verb: &str, wrong: &Wrong, usage: &str) -> i32 {
    let body = format!("{verb}: {}", wrong.message);
    match &wrong.word {
        Some(word) => crate::env::complain_with_usage(words, word, &body, &wrong.label, usage),
        // Nothing to point at: the line, and the usage under it, exactly as before.
        None => {
            eprintln!("{}{body}", crate::env::origin_now());
            eprintln!("{usage}");
        }
    }
    2
}

/// The strings to work on: **the operands, then the rows, then the input lines** — the first of
/// those there is.
///
/// Operands first is what lets a scalar verb open a pipeline: `text split : "$PATH"` has no stage
/// before it, and a verb that only read its input would have nothing to read. Rows before bytes is
/// what makes `ls | path extension` see the column rather than a rendering of the drawn table.
///
/// `columns` is the order to look for the string in a row, most specific first.
pub fn gather(
    operands: &[String],
    input: Option<&[Record]>,
    bytes: Option<&str>,
    columns: &[&str],
) -> Vec<String> {
    if !operands.is_empty() {
        return operands.to_vec();
    }
    if let Some(rows) = input.filter(|rows| !rows.is_empty()) {
        return rows.iter().map(|row| string_of(row, columns)).collect();
    }
    bytes
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

/// The string a row stands for.
///
/// The named columns in order, then whatever the first column is — because a row of one column has
/// an obvious string whatever it is called, and a verb that made you name it would be unusable
/// after `lines`.
pub fn string_of(record: &Record, columns: &[&str]) -> String {
    for name in columns {
        if let Some(value) = record.get(name) {
            return value.to_string();
        }
    }
    record
        .columns()
        .first()
        .and_then(|name| record.get(name))
        .map(Val::to_string)
        .unwrap_or_default()
}

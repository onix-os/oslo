//! What the scalar verbs share: finding the strings they were given.
//!
//! `text` and `path` are the same shape of tool — one column in, one column out, and the strings
//! can arrive three different ways. That arrival is the only part identical enough to be written
//! once; the subcommands and their flags have nothing in common and stay apart.

use super::super::value::{Record, Val};

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

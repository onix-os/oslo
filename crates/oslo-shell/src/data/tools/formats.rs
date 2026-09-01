//! Delimiter-separated text, in and out: `from csv`, `from tsv`, `to csv`, `to tsv`.
//!
//! ```text
//! cat report.csv | from csv | where 'amount > 100' | to json
//! ps | cols pid name | to csv > processes.csv
//! ```
//!
//! # Hand-rolled, and that is a decision about the build
//!
//! oslo ships as a **static musl binary with no C toolchain**, so every dependency is a question
//! about what the release can still be. CSV is a hundred lines and its edge cases are the ones this
//! module already had to settle for [`crate::data::render_transport`] — a separator inside a value,
//! a newline inside a value, and how you say a quote. Taking a crate for that would be borrowing
//! risk to avoid work already done.
//!
//! # The quoting rule
//!
//! RFC 4180, which is what every spreadsheet writes and reads: a field is quoted when it contains
//! the delimiter, a quote, a newline or a carriage return, and a quote inside a quoted field is
//! written twice. **A field is never quoted when it does not need to be**, so a plain table stays
//! greppable — the same reason `render_transport` stays line-oriented.
//!
//! # Not the same as `to text`
//!
//! `to text` is oslo's own transport: tab separated, backslash escapes, no header, meant to be read
//! back by `lines`. `to csv` is for somebody else's program, so it carries a header row and quotes
//! rather than escapes. Two audiences, two formats; conflating them is how a header ends up in a
//! hand-over.

use crate::data::{Record, Val, render_transport};

/// The character between fields.
pub fn delimiter(format: &str) -> Option<char> {
    match format {
        "csv" => Some(','),
        "tsv" => Some('\t'),
        _ => None,
    }
}

/// Rows from delimited text, taking the first line as the column names.
pub fn from_delimited(input: &str, delimiter: char) -> Result<Vec<Record>, String> {
    let mut records = split(input, delimiter)?;
    if records.is_empty() {
        return Ok(Vec::new());
    }
    let names = records.remove(0);
    if names.is_empty() {
        return Ok(Vec::new());
    }
    Ok(records
        .into_iter()
        .map(|cells| {
            let mut row = Record::new();
            for (name, cell) in names.iter().zip(cells) {
                row.set(name, scalar(&cell));
            }
            row
        })
        .collect())
}

/// Rows as delimited text, with a header row.
///
/// The header is the union of every row's columns, in first-seen order — the same rule the drawn
/// table follows, because rows are allowed to disagree about their columns and a reader of a CSV
/// needs one shape.
pub fn to_delimited(rows: &[Record], delimiter: char) -> String {
    let table = Val::table(rows.to_vec());
    let columns = table.columns();
    if columns.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    push_line(&mut out, columns.iter().map(String::as_str), delimiter);
    for row in rows {
        let cells: Vec<String> = columns
            .iter()
            .map(|name| row.get(name).map(render_transport).unwrap_or_default())
            .collect();
        push_line(&mut out, cells.iter().map(String::as_str), delimiter);
    }
    out
}

fn push_line<'a>(out: &mut String, cells: impl Iterator<Item = &'a str>, delimiter: char) {
    let quoted: Vec<String> = cells.map(|cell| quote(cell, delimiter)).collect();
    out.push_str(&quoted.join(&delimiter.to_string()));
    out.push('\n');
}

/// A field, quoted only when it has to be.
fn quote(text: &str, delimiter: char) -> String {
    let needs = text.contains(delimiter)
        || text.contains('"')
        || text.contains('\n')
        || text.contains('\r');
    match needs {
        false => text.to_string(),
        true => format!("\"{}\"", text.replace('"', "\"\"")),
    }
}

/// Split a whole document into rows of fields, honouring quotes.
///
/// A newline inside a quoted field does **not** end the record, which is the difference between a
/// parser and a `split('\n')` — and the case that quietly corrupts a spreadsheet export.
/// Whether `text` ends at a record boundary rather than inside a quoted field.
///
/// **Asked of the real parser rather than of a copy of its rules.** A streamed document is cut into
/// batches, and a cut inside `"one\ntwo"` would turn one record into two — silently, and only for
/// data that happens to quote a newline. The rules that decide it are not simple (a quote opens a
/// field only at its start, and `""` is an escaped quote inside one), so a second implementation of
/// them is a second thing to keep in step. This runs the same `split` and asks whether it was happy.
pub fn is_complete(text: &str, delimiter: char) -> bool {
    split(text, delimiter).is_ok()
}

fn split(input: &str, delimiter: char) -> Result<Vec<Vec<String>>, String> {
    let mut rows = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut cell = String::new();
    let mut quoted = false;
    let mut chars = input.chars().peekable();
    let mut any = false;

    while let Some(c) = chars.next() {
        any = true;
        if quoted {
            match c {
                '"' => match chars.peek() {
                    // A doubled quote is one quote inside the field.
                    Some('"') => {
                        cell.push('"');
                        chars.next();
                    }
                    _ => quoted = false,
                },
                other => cell.push(other),
            }
            continue;
        }
        match c {
            '"' if cell.is_empty() => quoted = true,
            c if c == delimiter => row.push(std::mem::take(&mut cell)),
            '\r' => {}
            '\n' => {
                row.push(std::mem::take(&mut cell));
                rows.push(std::mem::take(&mut row));
            }
            other => cell.push(other),
        }
    }
    if quoted {
        return Err("a quoted field is never closed".to_string());
    }
    // A document that does not end in a newline still has a last record.
    if !cell.is_empty() || !row.is_empty() {
        row.push(cell);
        rows.push(row);
    }
    if !any {
        return Ok(Vec::new());
    }
    Ok(rows)
}

/// A cell as the most specific kind it plainly is — `parse`'s rule, so a column of numbers compares
/// as numbers whichever bridge produced it.
fn scalar(text: &str) -> Val {
    let trimmed = text.trim();
    if let Ok(i) = trimmed.parse::<i64>() {
        return Val::Int(i);
    }
    if let Ok(f) = trimmed.parse::<f64>()
        && trimmed.contains('.')
    {
        return Val::Float(f);
    }
    Val::Str(trimmed.to_string())
}

#[cfg(test)]
#[path = "formats/tests.rs"]
mod tests;

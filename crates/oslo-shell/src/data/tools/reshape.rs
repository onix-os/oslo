//! Changing a stream's shape: which columns it has, and which rows.
//!
//! ```text
//! ls | reject mode | rename size bytes
//! ls | insert kb 'size / 1024' | sort-by -r kb
//! ps | enumerate | where 'index < 5'
//! docker inspect x | from json | flatten | cols State.Running
//! ```
//!
//! # Why these and not the other eighty
//!
//! nushell has ninety-odd filters. oslo cannot have ninety names, because **every name registered is
//! a name a POSIX script might already call** — that is the whole argument in `data::plan`, and it
//! is the budget everything here is rationed by. So the ones taken are the ones with no expression
//! in the existing vocabulary at all:
//!
//! | not taken | because |
//! |---|---|
//! | `take` | `first` already is it |
//! | `slice` | `skip` then `first` is it, without a range syntax to invent |
//! | `wrap`, `values`, `columns` | a stream is always rows here, so there is no list to wrap; and `columns` is one letter from `column`, which util-linux ships |
//! | `append`, `prepend`, `merge`, `join` | all need a *second* input, and the pipeline is a line |
//! | `move`, `transpose` | reorder rather than add; wanted less often than they cost |
//!
//! # The three that compute
//!
//! `insert`, `update` and `upsert` take a Lua expression evaluated per row, exactly as `where` and
//! `map` do — the row's columns are bound as globals, so `insert kb 'size / 1024'` reads the way it
//! looks. They differ **only** in what they do about a column that is already there:
//!
//! | verb | column present | column absent |
//! |---|---|---|
//! | `insert` | refuses | adds |
//! | `update` | replaces | refuses |
//! | `upsert` | replaces | adds |
//!
//! Two of the three refuse, and that is the point: `insert` on a column that exists is almost always
//! a typo for `update`, and silently overwriting is how a pipeline quietly loses a column.

use crate::data::path::Path;
use crate::data::{Record, Val};

/// Every column except the named ones, in the order they had.
///
/// The complement of `cols`, and worth its own name because the two are asked in different
/// situations: `cols` when you know the three columns you want, `reject` when you know the one you
/// do not. Top-level names only — a nested field is reached with `flatten` first.
pub fn reject(rows: &[Record], names: &[String]) -> Vec<Record> {
    rows.iter()
        .map(|row| {
            let mut out = row.clone();
            for name in names {
                out.remove(name);
            }
            out
        })
        .collect()
}

/// One column under a new name, in its own place.
pub fn rename(rows: &[Record], from: &str, to: &str) -> Vec<Record> {
    rows.iter()
        .map(|row| {
            let mut out = row.clone();
            out.rename(from, to);
            out
        })
        .collect()
}

/// What a computing verb does about a column that is already there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum When {
    /// `insert` — the column must not exist.
    Absent,
    /// `update` — the column must exist.
    Present,
    /// `upsert` — either.
    Either,
}

/// Set `column` on every row to what `values` computed for it.
///
/// `values` is one entry per row, as [`super::where_::compute`] answers it; `None` is a row whose
/// expression raised, and it is left exactly as it was rather than being given a hole.
pub fn assign(
    rows: &[Record],
    column: &str,
    values: &[Option<Val>],
    when: When,
) -> Result<Vec<Record>, String> {
    // Checked against the stream rather than per row, so the refusal is one message about the
    // pipeline rather than one per row about the data.
    let anywhere = rows.iter().any(|row| row.get(column).is_some());
    match when {
        When::Absent if anywhere => {
            return Err(format!(
                "insert: {column}: already a column; use update to replace it, or upsert for either"
            ));
        }
        When::Present if !anywhere && !rows.is_empty() => {
            return Err(format!(
                "update: {column}: no such column; use insert to add it, or upsert for either"
            ));
        }
        _ => {}
    }
    Ok(rows
        .iter()
        .zip(values.iter())
        .map(|(row, value)| match value {
            Some(value) => {
                let mut out = row.clone();
                out.set(column, value.clone());
                out
            }
            None => row.clone(),
        })
        .collect())
}

/// Nested records spread into columns named by their path.
///
/// `{ state = { running = true } }` becomes a column called `state.running`, which is the name
/// [`Path`] would have used to reach it — so `flatten | cols state.running` and
/// `cols state.running` name the same thing, and the flattened form is not a second vocabulary.
///
/// One level at a time is not enough for real documents, so this is recursive; a list is left
/// alone, because spreading `images.0` `images.1` into columns makes the column set depend on the
/// data, and two rows would stop having the same shape.
pub fn flatten(rows: &[Record]) -> Vec<Record> {
    rows.iter()
        .map(|row| {
            let mut out = Record::new();
            spread(&mut out, "", row);
            out
        })
        .collect()
}

fn spread(out: &mut Record, prefix: &str, row: &Record) {
    for (name, value) in row.columns().iter().zip(row.values()) {
        let full = match prefix.is_empty() {
            true => name.clone(),
            false => format!("{prefix}.{name}"),
        };
        match value {
            Val::Record(inner) => spread(out, &full, inner),
            other => out.set(&full, other.clone()),
        }
    }
}

/// The first row's values become the column names, and it stops being a row.
///
/// The other half of reading somebody else's table: `detect-columns` and `lines` produce whatever
/// they were given, and a program that printed its own header has put the names in row one.
pub fn headers(rows: &[Record]) -> Vec<Record> {
    let Some((first, rest)) = rows.split_first() else {
        return Vec::new();
    };
    let names: Vec<String> = first
        .values()
        .iter()
        .map(crate::data::render_transport)
        .collect();
    rest.iter()
        .map(|row| {
            let mut out = Record::new();
            for (i, value) in row.values().iter().enumerate() {
                // A row wider than the header keeps its extra cells under their old names, rather
                // than losing them: a ragged table is still data.
                let name = names
                    .get(i)
                    .cloned()
                    .or_else(|| row.columns().get(i).cloned())
                    .unwrap_or_else(|| i.to_string());
                out.set(&name, value.clone());
            }
            out
        })
        .collect()
}

/// All but the first `n` rows — the other end of `first`.
pub fn skip(rows: &[Record], n: usize) -> Vec<Record> {
    rows.iter().skip(n).cloned().collect()
}

/// Every `n`th row, starting with the first.
pub fn every(rows: &[Record], n: usize) -> Vec<Record> {
    if n == 0 {
        return Vec::new();
    }
    rows.iter().step_by(n).cloned().collect()
}

/// Each row with its position, counted from zero.
///
/// The column goes **first**, because it is what you are about to read, and last would put it after
/// however many columns the producer happened to have.
pub fn enumerate(rows: &[Record]) -> Vec<Record> {
    rows.iter()
        .enumerate()
        .map(|(i, row)| {
            let mut out = Record::from_pairs([("index", Val::Int(i as i64))]);
            for (name, value) in row.columns().iter().zip(row.values()) {
                out.set(name, value.clone());
            }
            out
        })
        .collect()
}

/// Rows that have something in `column` — or, with no column, in every column.
///
/// "Something" is neither absent nor [`Val::Null`]. A [`Val::Error`] survives, because it *is*
/// something: the cell failed and the row is entitled to say so, and dropping it here would hide
/// exactly the rows worth looking at.
pub fn compact(rows: &[Record], column: Option<&str>) -> Vec<Record> {
    let path = column.map(Path::parse);
    rows.iter()
        .filter(|row| match &path {
            Some(path) => !matches!(path.get_or_absent(row), None | Some(Val::Null)),
            None => !row.values().iter().any(|v| matches!(v, Val::Null)),
        })
        .cloned()
        .collect()
}

/// Fill an absent or null `column` with `value`, leaving every other row alone.
pub fn default(rows: &[Record], column: &str, value: &Val) -> Vec<Record> {
    rows.iter()
        .map(|row| match row.get(column) {
            Some(Val::Null) | None => {
                let mut out = row.clone();
                out.set(column, value.clone());
                out
            }
            Some(_) => row.clone(),
        })
        .collect()
}

#[cfg(test)]
#[path = "reshape/tests.rs"]
mod tests;

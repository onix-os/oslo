//! `explore` — the rows, on the alternate screen, where you can move around them.
//!
//! ```text
//! ps | where 'rss > 1e8' | explore
//! docker inspect x | from json | explore
//! ```
//!
//! The verb itself is four lines: everything interesting is in `oslo_ui::explore`, and everything
//! *here* is the translation between the shell's `Val` and the plain [`Sheet`] that crate can see.
//!
//! # A nested cell is a table of its own
//!
//! The drawn table says `<3 items>` because a row that is two lines is not a row. That is the right
//! answer for a transcript and a dead end when the thing you wanted was inside the cell — so here
//! the same summary is a door: Enter opens it as a table, Backspace comes back. The summary itself
//! is `value::one_line`, the function the drawn table already uses, because a cell that read
//! `<3 items>` in one face and `[3]` in the other would be two vocabularies for one thing.
//!
//! A **record** opens as `field`/`value`, which is how a record is read — down, not across. A
//! **list of records** opens as the table it already is. A list of anything else opens as one
//! `value` column, one row per item.

use crate::data::{Record, Val};
use oslo_ui::explore::{Cell, Sheet};

/// The rows of a pipeline, as a sheet to explore.
pub fn sheet(title: &str, rows: &[Record]) -> Sheet {
    let columns = union(rows);
    let cells: Vec<Vec<Cell>> = rows
        .iter()
        .map(|row| {
            columns
                .iter()
                .map(|name| match row.get(name) {
                    Some(value) => cell(name, value),
                    None => Cell::Flat(String::new()),
                })
                .collect()
        })
        .collect();
    made(title, columns, cells)
}

/// A sheet, with its alignment worked out from the cells it holds.
///
/// Every sheet is built through here — the nested ones too — so no level of the descent can be
/// assembled without deciding which of its columns are numbers.
fn made(title: &str, columns: Vec<String>, rows: Vec<Vec<Cell>>) -> Sheet {
    Sheet {
        title: title.to_string(),
        numeric: aligned(&rows, columns.len()),
        columns,
        rows,
    }
}

/// Which columns are drawn right-aligned, asked of the renderer that already decides it.
///
/// The drawn table and the viewer must agree — a column that changed alignment when you opened it
/// would look like different data. So the rule lives once, in `value::drawn`, and this hands it the
/// cells as they will appear.
fn aligned(cells: &[Vec<Cell>], count: usize) -> Vec<bool> {
    let text: Vec<Vec<String>> = cells
        .iter()
        .map(|row| row.iter().map(|cell| cell.text().to_string()).collect())
        .collect();
    // The null placeholder is the drawn table's, and a sheet has none of its own: an absent cell is
    // already an empty string by the time it gets here.
    crate::data::value::numeric_columns(&text, count, "")
}

/// Every column any row has, in the order they were first seen.
fn union(rows: &[Record]) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for row in rows {
        for name in row.columns() {
            if !names.iter().any(|seen| seen == name) {
                names.push(name.clone());
            }
        }
    }
    names
}

/// One cell: text, or a door into a table.
///
/// An **empty** list or record is flat rather than nested, and deliberately: `<0 items>` opens onto
/// a level with nothing on it, which reads as the viewer having broken rather than as the cell
/// having been empty all along.
fn cell(name: &str, value: &Val) -> Cell {
    let summary = crate::data::value::one_line(value);
    match value {
        Val::List(items) if !items.is_empty() => Cell::Nested {
            summary,
            sheet: Box::new(list(name, items)),
        },
        Val::Record(record) if !record.columns().is_empty() => Cell::Nested {
            summary,
            sheet: Box::new(fields(name, record)),
        },
        _ => Cell::Flat(summary),
    }
}

/// A record, read down: one row per field.
fn fields(name: &str, record: &Record) -> Sheet {
    made(
        name,
        vec!["field".to_string(), "value".to_string()],
        record
            .columns()
            .iter()
            .zip(record.values())
            .map(|(field, value)| vec![Cell::Flat(field.clone()), cell(field, value)])
            .collect(),
    )
}

/// A list. Records make a table of their own columns; anything else makes one `value` column.
fn list(name: &str, items: &[Val]) -> Sheet {
    let records: Vec<Record> = items
        .iter()
        .filter_map(|item| match item {
            Val::Record(record) => Some(record.clone()),
            _ => None,
        })
        .collect();
    if records.len() == items.len() {
        return sheet(name, &records);
    }
    made(
        name,
        vec!["value".to_string()],
        items.iter().map(|item| vec![cell(name, item)]).collect(),
    )
}

#[cfg(test)]
#[path = "explore/tests.rs"]
mod tests;

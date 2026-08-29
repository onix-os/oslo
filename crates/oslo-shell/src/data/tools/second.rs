//! The verbs that need a **second** stream: `lookup`, `append`, `merge`.
//!
//! ```text
//! ls | lookup 'sh.stat("Cargo.toml")' name
//! ps | append 'oslo.rows.from_json(saved)'
//! ls | merge 'extra_columns()'
//! ```
//!
//! # The shape problem, and the shape chosen
//!
//! `summarise` said it plainly and it was right: a join "needs a *second* input stream, and oslo's
//! pipeline is a line — there is no shape for 'and also read this'". Three ways to give it one were
//! on the table:
//!
//! | shape | why not, or why |
//! |---|---|
//! | `lookup (ls) name` — a command substitution evaluated as a *structured* pipeline | the right answer eventually, but the planner cannot see inside an operand today, so it would run on the byte path and arrive as text |
//! | `ls \| save-rows x` then `lookup x name` — a named saved stream | a new concept, a new name, and a lifetime to explain |
//! | **`lookup '<lua>' name`** — the other side is a Lua expression | works now, costs no planner change, and the language is one the user already has at the prompt |
//!
//! The third is what these use. It is not the prettiest of the three and it is not pretending to
//! be: it is the one that does not require inventing a second way for a pipeline to fork. When the
//! planner can recurse into an operand, the first becomes possible and this stays as what it always
//! was — the escape hatch, in the language the filter is already written in.
//!
//! # `join` is not the name
//!
//! `join` is **POSIX.1** and coreutils ships it. A rows producer piped into a name a script already
//! calls is exactly the defect `uniq` had, and the whole vocabulary is disjoint from real commands
//! on purpose. `lookup` is what the verb is usually for anyway — enriching rows from another table.
//!
//! # A collision keeps both sides
//!
//! When both sides have a column of the same name, the left keeps the name and the right arrives as
//! `<name>_2`. Overwriting would lose data silently and skipping would lose it loudly; a suffix is
//! the only one of the three where nothing disappears.

use crate::data::path::Path;
use crate::data::{Record, render_transport};

/// The value a row joins on, compared as a value rather than as a rendering.
fn key_of(row: &Record, on: &Path) -> Option<String> {
    on.get_or_absent(row).map(render_transport)
}

/// Rows from the left, each carrying the matching row from the right.
///
/// **Inner by default**: a left row with no match does not survive, because a `lookup` that quietly
/// kept unmatched rows with empty columns would make "did this match?" unanswerable downstream.
/// `keep_unmatched` is the left-outer form, for when the question is which rows *failed* to match.
///
/// A left row matching several right rows produces several rows — that is what a join is, and
/// silently taking the first would be a different operation wearing this one's name.
pub fn lookup(left: &[Record], right: &[Record], on: &str, keep_unmatched: bool) -> Vec<Record> {
    let path = Path::parse(on);
    // Indexed once, so a join is a scan of the left side rather than a scan per row.
    let mut index: std::collections::HashMap<String, Vec<&Record>> =
        std::collections::HashMap::new();
    for row in right {
        if let Some(key) = key_of(row, &path) {
            index.entry(key).or_default().push(row);
        }
    }

    let mut out = Vec::new();
    for row in left {
        let matches = key_of(row, &path).and_then(|key| index.get(&key));
        match matches {
            Some(found) => out.extend(found.iter().map(|other| joined(row, other, on))),
            None if keep_unmatched => out.push(row.clone()),
            None => {}
        }
    }
    out
}

/// One row from two, with the right side's colliding columns suffixed.
fn joined(left: &Record, right: &Record, on: &str) -> Record {
    let mut out = left.clone();
    for (name, value) in right.columns().iter().zip(right.values()) {
        // The key is the same value on both sides by construction, so it is not carried twice.
        if name == on {
            continue;
        }
        let name = match left.get(name).is_some() {
            true => format!("{name}_2"),
            false => name.clone(),
        };
        out.set(&name, value.clone());
    }
    out
}

/// One stream after another.
///
/// Rows are allowed to disagree about their columns, so nothing is reconciled here — the drawn
/// table takes the union, as it always does, and a column only one side has is simply absent on the
/// other. That is the same rule `ps` already relies on.
pub fn append(left: &[Record], right: &[Record]) -> Vec<Record> {
    let mut out = left.to_vec();
    out.extend(right.iter().cloned());
    out
}

/// Two streams paired by position, the right side's columns winning a collision.
///
/// **Position, not a key** — that is the difference from [`lookup`], and it is why this is a
/// separate verb rather than a flag: merging by position is a statement that the two streams are
/// already in the same order, which is a thing only the person typing it knows.
///
/// The result is as long as the *left* stream: extra rows on the right have no row to merge into,
/// and inventing one would silently change how many rows the pipeline has.
pub fn merge(left: &[Record], right: &[Record]) -> Vec<Record> {
    left.iter()
        .enumerate()
        .map(|(i, row)| {
            let mut out = row.clone();
            if let Some(other) = right.get(i) {
                for (name, value) in other.columns().iter().zip(other.values()) {
                    out.set(name, value.clone());
                }
            }
            out
        })
        .collect()
}

#[cfg(test)]
#[path = "second/tests.rs"]
mod tests;

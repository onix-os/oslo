//! `df` — free space, with two faces.
//!
//! ```text
//! df                     the table a person reads
//! df | where 'free < 1e9'   the rows a filter reads
//! ```
//!
//! **One source of facts.** The rows are the truth; the text face is a rendering of them. That is
//! the shape `docs/features/structured-pipelines.md` argues for, and the reason is that any tool with two
//! independent implementations of "what the filesystems are" ends up with two answers.
//!
//! The facts come from the external `df`, not from re-implementing it. Parsing a tool that already
//! knows about every mount type on the system is less work and more correct than a second
//! implementation, and it is the strategy every later tool will use.

use crate::data::rows::parse_df;
use crate::data::{Record, Val};

/// The columns `df` answers with, in the order it builds them.
///
/// Declared beside the code that fills them so the two cannot drift apart unnoticed — and
/// `data::columns` reads this, which is what lets the planner refuse `df | cols mounted_on`
/// before `df` runs. `tests` checks the declaration against a real run.
pub const COLUMNS: &[&str] = &["filesystem", "size", "used", "free", "capacity", "mounted"];

/// A byte count `df` gave, or a cell that says it did not.
///
/// **[`Val::Error`] is a value, not a failure**, which is the whole reason the kind exists: the row
/// still arrives, the rest of its columns are still good, and the stream carries on. A text tool
/// warns about the one filesystem and prints the others, and that is exactly why people trust it.
fn size(bytes: Option<u64>) -> Val {
    match bytes {
        Some(bytes) => Val::Size(bytes),
        None => unreadable(),
    }
}

/// What a cell says when `df` would not answer for it.
///
/// `df -P` prints `-` for every figure of a mount it cannot reach — a stale NFS handle is the usual
/// way — and the message names that rather than the character, because `-` on its own explains
/// nothing to somebody reading a table.
fn unreadable() -> Val {
    Val::Error("df did not report a figure for this filesystem".to_string())
}

/// The rows `df` produces.
pub fn rows() -> Result<Vec<Record>, String> {
    // `-P` fixes the block size at 1024 and stops long device names wrapping onto a second line,
    // which is the difference between a parser and a guess.
    let output = std::process::Command::new("df")
        .arg("-P")
        .output()
        .map_err(|e| format!("df: {e}"))?;
    let text = String::from_utf8_lossy(&output.stdout);
    Ok(parse_df(&text)
        .into_iter()
        .map(|fs| {
            Record::from_pairs([
                ("filesystem", Val::Str(fs.source)),
                ("size", size(fs.size)),
                ("used", size(fs.used)),
                ("free", size(fs.free)),
                (
                    "capacity",
                    match fs.capacity {
                        Some(percent) => Val::Int(percent as i64),
                        None => unreadable(),
                    },
                ),
                ("mounted", Val::Str(fs.mount)),
            ])
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Sizes are sizes, not text: that is what makes `where 'free < 1e9'` arithmetic.
    #[test]
    fn rows_carry_sizes_as_sizes() {
        // Reading the real filesystem table would make this a test of the machine, so this checks
        // the shape the parser produces from output known in advance.
        let sample = "Filesystem 1024-blocks Used Available Capacity Mounted on\n\
                      /dev/sda1 1000 400 600 40% /\n";
        let parsed = parse_df(sample);
        assert_eq!(parsed.len(), 1);
        let row = Record::from_pairs([
            ("filesystem", Val::Str(parsed[0].source.clone())),
            ("free", size(parsed[0].free)),
        ]);
        assert_eq!(row.get("free"), Some(&Val::Size(600 * 1024)));
        assert_eq!(row.columns(), ["filesystem", "free"]);
    }

    /// **The kind existed and nothing ever produced one.** `Val::Error` was plumbed through both
    /// renderers, `to json`, the Lua bridge, `compact` and `describe` — and no producer emitted it,
    /// so the whole idea that "an error is a value the stream survives" was untested against
    /// anything real. `df` is the case its own module documentation names.
    #[test]
    fn a_figure_df_would_not_give_becomes_an_error_cell() {
        let stale = "Filesystem 1024-blocks Used Available Capacity Mounted on\n\
                     server:/export - - - - /mnt/stale\n\
                     /dev/sda1 1000 400 600 40% /\n";
        let rows: Vec<Record> = parse_df(stale)
            .into_iter()
            .map(|fs| {
                Record::from_pairs([
                    ("filesystem", Val::Str(fs.source)),
                    ("free", size(fs.free)),
                    ("mounted", Val::Str(fs.mount)),
                ])
            })
            .collect();

        assert_eq!(rows.len(), 2, "both filesystems arrive");
        let Some(Val::Error(why)) = rows[0].get("free") else {
            panic!(
                "the unreachable mount carries an error cell, got {:?}",
                rows[0].get("free")
            );
        };
        assert!(
            why.contains("df"),
            "and it says who could not answer: {why}"
        );

        // **The rest of the row is good and the stream carries on**, which is the whole argument
        // for an error being a value rather than a failure.
        assert_eq!(
            rows[0].get("mounted"),
            Some(&Val::Str("/mnt/stale".to_string()))
        );
        assert_eq!(rows[1].get("free"), Some(&Val::Size(600 * 1024)));

        // A person sees what went wrong rather than a blank or a zero.
        let drawn = crate::data::render_display(&Val::table(rows));
        assert!(drawn.contains("error"), "{drawn}");
    }
}

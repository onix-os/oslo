//! `df` — free space, with two faces.
//!
//! ```text
//! df                     the table a person reads
//! df | where 'free < 1e9'   the rows a filter reads
//! ```
//!
//! **One source of facts.** The rows are the truth; the text face is a rendering of them. That is
//! the shape `docs/built-in-tools.md` argues for, and the reason is that any tool with two
//! independent implementations of "what the filesystems are" ends up with two answers.
//!
//! The facts come from the external `df`, not from re-implementing it. Parsing a tool that already
//! knows about every mount type on the system is less work and more correct than a second
//! implementation, and it is the strategy every later tool will use.

use crate::data::rows::parse_df;
use crate::data::{Record, Val};

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
                ("size", Val::Size(fs.size)),
                ("used", Val::Size(fs.used)),
                ("free", Val::Size(fs.free)),
                ("capacity", Val::Int(fs.capacity as i64)),
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
            ("free", Val::Size(parsed[0].free)),
        ]);
        assert_eq!(row.get("free"), Some(&Val::Size(600 * 1024)));
        assert_eq!(row.columns(), ["filesystem", "free"]);
    }
}

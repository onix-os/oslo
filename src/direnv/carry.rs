//! The undo record, carried in the environment so a child shell can put it back.
//!
//! # Why this exists, when the module note says it deliberately did not
//!
//! Keeping the state in memory is right for one process, and wrong the moment there are two. A
//! directory environment has to write to the real `environ` — that is the whole point, since a
//! child's environment comes from `execve` reading it — and everything spawned from a shell
//! standing in a project therefore inherits that project's variables. A new pane, a nested `oslo`,
//! an editor's terminal: each starts with `loaded: None`, has no idea it inherited anything, and
//! so can never unload it. It carries one repository's `$PATH` into every directory it visits, for
//! the rest of its life, and no amount of `cd` will shift it.
//!
//! That is what direnv's `DIRENV_DIFF` is for, and skipping it was the mistake. This is the same
//! idea: the record of *what to put back* travels with the variables it describes, so whoever ends
//! up holding them can give them back.
//!
//! # The format
//!
//! Length-prefixed fields, `<bytes>:<content>`, after a version byte. No escaping and no
//! separators to collide with, because a variable's value is arbitrary text and every scheme that
//! needed quoting got it wrong on the value that contained the quote. A NUL cannot appear — the
//! environment could not carry it either.
//!
//! ```text
//! 1 4:/tmp 3:FOO 1:1 3:bar 5:EMPTY 1:u 0:
//!   └owner  └name └flag └value      └ was unset before, so unset it again
//! ```

use super::diff::Diff;
use std::path::{Path, PathBuf};

/// The variable this is carried in. Exported, like the variables it describes.
pub const NAME: &str = "OSLO_DIRENV";

/// Everything a shell needs to undo an environment it did not load.
pub struct Carried {
    pub owner: PathBuf,
    pub undo: Diff<(String, bool)>,
}

/// The record for a load of `owner`, whose undo is `undo`.
pub fn encode(owner: &Path, undo: &Diff<(String, bool)>) -> String {
    let mut out = String::from("1");
    field(&mut out, &owner.to_string_lossy());
    for (name, was) in undo.to_apply() {
        field(&mut out, name);
        match was {
            Some((value, exported)) => {
                field(&mut out, if *exported { "1" } else { "0" });
                field(&mut out, value);
            }
            // No value to carry, and a length of zero says so without a second encoding.
            None => {
                field(&mut out, "u");
                field(&mut out, "");
            }
        }
    }
    out
}

/// Read a record back, or `None` if it is not one.
///
/// **Anything malformed answers `None` rather than a partial record.** This arrives from the
/// environment, which is to say from anywhere, and applying half of an undo would leave the shell
/// in a state neither the parent nor the child could describe.
pub fn decode(text: &str) -> Option<Carried> {
    let rest = text.strip_prefix('1')?;
    let mut fields = Fields { rest };
    let owner = PathBuf::from(fields.next()?);
    let mut pairs = Vec::new();
    while !fields.rest.is_empty() {
        let name = fields.next()?;
        let flag = fields.next()?;
        let value = fields.next()?;
        let was = match flag {
            "u" => None,
            "1" => Some((value.to_string(), true)),
            "0" => Some((value.to_string(), false)),
            _ => return None,
        };
        pairs.push((name.to_string(), was));
    }
    Some(Carried {
        owner,
        undo: Diff::to_restore(pairs),
    })
}

fn field(out: &mut String, text: &str) {
    out.push(' ');
    out.push_str(&text.len().to_string());
    out.push(':');
    out.push_str(text);
}

/// The remaining fields of a record.
struct Fields<'a> {
    rest: &'a str,
}

impl<'a> Fields<'a> {
    fn next(&mut self) -> Option<&'a str> {
        let rest = self.rest.strip_prefix(' ')?;
        let (len, after) = rest.split_once(':')?;
        let len: usize = len.parse().ok()?;
        // Byte length, and the slice has to land on a character boundary — a truncated record
        // could otherwise panic here rather than being refused.
        if after.len() < len || !after.is_char_boundary(len) {
            return None;
        }
        let (value, remaining) = after.split_at(len);
        self.rest = remaining;
        Some(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(pairs: &[(&str, Option<(&str, bool)>)]) -> Diff<(String, bool)> {
        Diff::to_restore(
            pairs
                .iter()
                .map(|(name, was)| (name.to_string(), was.map(|(v, e)| (v.to_string(), e))))
                .collect(),
        )
    }

    fn round_trip(owner: &str, pairs: &[(&str, Option<(&str, bool)>)]) -> Carried {
        let encoded = encode(Path::new(owner), &record(pairs));
        decode(&encoded).expect("its own output")
    }

    /// What went out comes back, values and export flags together.
    #[test]
    fn a_record_survives_the_environment() {
        let back = round_trip(
            "/home/me/project",
            &[
                ("PATH", Some(("/usr/bin", true))),
                ("LOCAL", Some(("kept", false))),
                ("NEW", None),
            ],
        );
        assert_eq!(back.owner, PathBuf::from("/home/me/project"));
        assert_eq!(
            back.undo.to_apply(),
            vec![
                ("LOCAL", Some(&("kept".to_string(), false))),
                ("NEW", None),
                ("PATH", Some(&("/usr/bin".to_string(), true))),
            ]
        );
    }

    /// **The export flag travels with the value.** A variable that was shell-local before the
    /// directory exported it has to come back local, or the shell that adopts the record hands
    /// every later child a variable that was never in its environment.
    #[test]
    fn the_export_flag_is_part_of_the_record() {
        let back = round_trip("/p", &[("A", Some(("x", true))), ("B", Some(("x", false)))]);
        let applied = back.undo.to_apply();
        assert_eq!(applied[0].1.map(|(_, e)| *e), Some(true), "{applied:?}");
        assert_eq!(applied[1].1.map(|(_, e)| *e), Some(false), "{applied:?}");
    }

    /// A value is arbitrary text. Length prefixes rather than separators is what makes that true
    /// rather than nearly true — every scheme that needed quoting got it wrong on the value that
    /// contained the quote.
    #[test]
    fn a_value_may_contain_anything() {
        let awkward = " 12: :\"'\\\n\ttrailing ";
        let back = round_trip("/dir with spaces", &[("V", Some((awkward, true)))]);
        assert_eq!(back.owner, PathBuf::from("/dir with spaces"));
        assert_eq!(
            back.undo.to_apply()[0].1.map(|(v, _)| v.as_str()),
            Some(awkward)
        );
    }

    /// An empty value is not the same as no value: restoring `FOO` to `""` leaves an empty
    /// variable, which `[ -n "$FOO" ]` reads differently from the absence that was there before.
    #[test]
    fn empty_and_absent_stay_different() {
        let back = round_trip("/p", &[("EMPTY", Some(("", true))), ("GONE", None)]);
        let applied = back.undo.to_apply();
        assert_eq!(applied[0].1.map(|(v, _)| v.as_str()), Some(""));
        assert_eq!(applied[1].1, None);
    }

    /// Anything that is not a record is refused whole. It arrives from the environment, which is
    /// to say from anywhere, and half an undo is a state nobody can describe.
    #[test]
    fn rubbish_is_refused_rather_than_half_read() {
        for bad in [
            "",
            "2 4:/tmp",               // a version this does not speak
            "1 4:/tmp 9:short",       // a length past the end
            "1 4:/tmp 3:FOO",         // a name with no flag or value
            "1 4:/tmp 3:FOO 1:x 1:v", // a flag that means nothing
            "1 x:/tmp",               // a length that is not a number
            "14:/tmp",                // no separator
        ] {
            assert!(decode(bad).is_none(), "{bad:?} should be refused");
        }
    }

    /// A multibyte value cannot be cut in half by a length that lands inside it.
    #[test]
    fn a_truncated_multibyte_value_is_refused() {
        let good = encode(Path::new("/p"), &record(&[("V", Some(("é→", true)))]));
        assert!(decode(&good).is_some());
        // The same record with the value's byte length reduced by one, which lands mid-character.
        let cut = good.replace(" 5:é→", " 4:é→");
        assert!(decode(&cut).is_none(), "{cut:?}");
    }

    /// Nothing loaded is still a valid record — the owner with no variables after it.
    #[test]
    fn a_load_that_changed_nothing_round_trips() {
        let back = round_trip("/p", &[]);
        assert_eq!(back.owner, PathBuf::from("/p"));
        assert!(back.undo.is_empty());
    }
}

//! Reaching into a row: `metadata.name`, `spec.containers.0.image`.
//!
//! ```text
//! kubectl get pods -o json | from json | get metadata.name
//! docker inspect x | from json | cols Id 'State.Running'
//! ```
//!
//! # Why this is a primitive and not a feature of one verb
//!
//! `from json` has always produced nested cells — a JSON object inside a column is a
//! [`Val::Record`], and a JSON array is a [`Val::List`]. Nothing in the shell could reach inside
//! one. `get metadata` answered the whole record rendered as a cell, and the only way to the name
//! inside it was to leave the pipeline for Lua.
//!
//! The alternative to a shared path type is each verb growing its own descent rule, and then
//! `cols` and `sort-by` disagreeing about what `a.0.b` means. One parser, one resolver, one set of
//! mistakes.
//!
//! # An exact column wins
//!
//! **A column really can be called `a.b`.** `parse '{a.b}:{x}'` makes one, and so does any JSON
//! document with a dotted key. So resolution asks for the literal column first and only descends
//! when there is no such column — a producer that named a column `a.b` meant it, and guessing
//! otherwise would make its data unreachable with no way to ask for it.
//!
//! # Missing, and `?`
//!
//! A step that is not there is an **error**, because a typo that silently answers nothing is the
//! failure this project keeps finding: `cols mispelled` producing a stream of empty rows is worse
//! than a refusal. A trailing `?` on a step says the absence is expected and answers nothing
//! instead — nushell spells it the same way, and it is what makes a ragged document usable:
//!
//! ```text
//! from json | get 'status.reason?'      a pod with no reason yields nothing, not an error
//! ```

use crate::data::{Record, Val};

/// One step of a descent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    name: String,
    /// Whether a `?` followed it: absence here is an answer, not a mistake.
    optional: bool,
}

/// A parsed path into a row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Path {
    /// The text exactly as it was written, which is both the literal-column candidate and the name
    /// a verb gives the column it produces.
    literal: String,
    steps: Vec<Step>,
}

impl Path {
    /// Read a path. Any word is one: a name with no dots is a path of one step, which is what makes
    /// every existing `get name` keep working unchanged.
    pub fn parse(text: &str) -> Path {
        let steps = text
            .split('.')
            .map(|part| match part.strip_suffix('?') {
                Some(name) => Step {
                    name: name.to_string(),
                    optional: true,
                },
                None => Step {
                    name: part.to_string(),
                    optional: false,
                },
            })
            .collect();
        Path {
            literal: text.to_string(),
            steps,
        }
    }

    /// The path as written — the name a verb gives the column it produces, so that `cols a.b`
    /// followed by `get a.b` finds what the first one made.
    pub fn literal(&self) -> &str {
        &self.literal
    }

    /// Whether this is a single plain step, which is the overwhelmingly common case and the one
    /// worth not allocating for.
    pub fn is_plain(&self) -> bool {
        self.steps.len() == 1 && !self.steps[0].optional
    }

    /// The first step and whether it was optional — the only part of a path that can be judged
    /// against a *declared* column set.
    ///
    /// Everything after it is a question about data: whether `metadata` holds a record with a
    /// `name` in it cannot be known before the rows exist. See [`crate::data::columns::Columns`],
    /// which uses this to be generous on purpose.
    pub fn first_step(&self) -> Option<(&str, bool)> {
        self.steps
            .first()
            .map(|step| (step.name.as_str(), step.optional))
    }

    /// The value this path reaches in `row`, or why it could not.
    ///
    /// `Ok(None)` is "not there, and the path said that was allowed". `Err` is "not there, and
    /// nothing said that was expected".
    pub fn get<'a>(&self, row: &'a Record) -> Result<Option<&'a Val>, String> {
        // **The literal column first.** A column may genuinely be called `a.b`.
        if let Some(found) = row.get(&self.literal) {
            return Ok(Some(found));
        }
        let Some((first, rest)) = self.steps.split_first() else {
            return Ok(None);
        };
        let Some(mut current) = row.get(&first.name) else {
            return match first.optional {
                true => Ok(None),
                false => Err(first.name.clone()),
            };
        };
        for step in rest {
            match descend(current, &step.name) {
                Some(next) => current = next,
                None => {
                    return match step.optional {
                        true => Ok(None),
                        false => Err(step.name.clone()),
                    };
                }
            }
        }
        Ok(Some(current))
    }

    /// The value, with a missing step treated as absent rather than as a mistake.
    ///
    /// For the callers that are already deciding what to do about a gap — `cols` keeps a column
    /// only some rows have, and `sort-by` orders a row that lacks the key rather than refusing it.
    pub fn get_or_absent<'a>(&self, row: &'a Record) -> Option<&'a Val> {
        self.get(row).ok().flatten()
    }
}

/// One step into a value: a field of a record, or an index of a list.
///
/// Which one it is comes from **the value, not the path**, so a path never has to say whether `0`
/// means a column called `0` or the first element. A record is asked for the name; a list parses it
/// as a position.
fn descend<'a>(value: &'a Val, step: &str) -> Option<&'a Val> {
    match value {
        Val::Record(record) => record.get(step),
        Val::List(items) => {
            let at: usize = step.parse().ok()?;
            items.get(at)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nested() -> Record {
        let inner =
            Record::from_pairs([("name", Val::Str("web".into())), ("port", Val::Int(8080))]);
        Record::from_pairs([
            ("kind", Val::Str("Pod".into())),
            ("metadata", Val::Record(inner)),
            (
                "images",
                Val::List(vec![Val::Str("a:1".into()), Val::Str("b:2".into())]),
            ),
        ])
    }

    /// A plain name is a path of one step, so everything written before paths existed still works.
    #[test]
    fn a_plain_name_is_still_a_name() {
        let path = Path::parse("kind");
        assert!(path.is_plain());
        assert_eq!(path.get(&nested()).unwrap(), Some(&Val::Str("Pod".into())));
    }

    /// Into a record by name, and into a list by position — decided by the value, not the path.
    #[test]
    fn a_path_descends_records_and_lists() {
        let row = nested();
        assert_eq!(
            Path::parse("metadata.name").get(&row).unwrap(),
            Some(&Val::Str("web".into()))
        );
        assert_eq!(
            Path::parse("metadata.port").get(&row).unwrap(),
            Some(&Val::Int(8080))
        );
        assert_eq!(
            Path::parse("images.1").get(&row).unwrap(),
            Some(&Val::Str("b:2".into()))
        );
    }

    /// **A column really can be called `a.b`**, and asking for it must find it rather than
    /// descending into a record that does not exist.
    #[test]
    fn an_exact_column_wins_over_a_descent() {
        let row = Record::from_pairs([
            ("a.b", Val::Str("the literal column".into())),
            ("a", Val::Record(Record::from_pairs([("b", Val::Int(9))]))),
        ]);
        assert_eq!(
            Path::parse("a.b").get(&row).unwrap(),
            Some(&Val::Str("the literal column".into())),
            "the column that is actually called a.b"
        );
    }

    /// A step that is not there is a mistake, because a typo answering nothing is worse than a
    /// refusal — unless the path said the absence was expected.
    #[test]
    fn a_missing_step_is_an_error_unless_it_is_optional() {
        let row = nested();
        assert_eq!(Path::parse("metadata.nope").get(&row), Err("nope".into()));
        assert_eq!(Path::parse("nope.name").get(&row), Err("nope".into()));
        assert_eq!(Path::parse("metadata.nope?").get(&row), Ok(None));
        assert_eq!(Path::parse("nope?.name").get(&row), Ok(None));
    }

    /// Descending into something that is neither a record nor a list stops, rather than pretending.
    #[test]
    fn a_scalar_has_nothing_inside_it() {
        assert_eq!(Path::parse("kind.name").get(&nested()), Err("name".into()));
        assert_eq!(Path::parse("images.9").get(&nested()), Err("9".into()));
        assert_eq!(Path::parse("images.x").get(&nested()), Err("x".into()));
    }

    /// The written form is what a verb names the column it produces, so `cols a.b | get a.b` finds
    /// what the first one made.
    #[test]
    fn the_literal_is_the_name_of_what_it_produces() {
        assert_eq!(Path::parse("metadata.name").literal(), "metadata.name");
    }
}

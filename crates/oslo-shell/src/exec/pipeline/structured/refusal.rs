//! Refusing a column the stream cannot have, before anything runs.

use super::*;

/// The operands of `name` that must name a column the stream **already has**.
///
/// Only these. A verb that *creates* a column (`insert`, `default`) names one that is supposed to be
/// absent, and a verb whose operand is an expression (`where`, `map`, `reduce`) names no column at
/// all — checking either would refuse working pipelines, which is the one thing this must not do.
fn column_operands<'a>(name: &str, words: &'a [String]) -> Vec<&'a str> {
    let rest = || words.iter().skip(1).map(String::as_str);
    let first = || words.get(1).map(String::as_str).into_iter().collect();
    match name {
        // Every operand is a column.
        "cols" | "reject" => rest().collect(),
        // Flags first, then keys; `--` ends them, as `sort_operands` reads it.
        "sort-by" => {
            let mut done = false;
            rest()
                .filter(|word| {
                    if done || !word.starts_with('-') || *word == "-" {
                        return true;
                    }
                    done |= *word == "--";
                    false
                })
                .collect()
        }
        // The first operand, and only it.
        "get" | "group-by" | "stats" | "histogram" | "update" => first(),
        // Optional: absent means "by the whole row", which names nothing.
        "distinct" | "compact" => first(),
        // The old name has to be there; the new one must not be.
        "rename" => first(),
        // The key, which sits after the Lua expression — and after `--keep` when it is given.
        "lookup" => {
            let at = if words.get(1).is_some_and(|w| w == "--keep") {
                3
            } else {
                2
            };
            words.get(at).map(String::as_str).into_iter().collect()
        }
        _ => Vec::new(),
    }
}

/// Refuse a column no stage can be carrying, **before any stage runs**.
///
/// This is `data::plan`'s question asked one level down: the pipe already decides what shape crosses
/// an edge, and now it decides whether what a stage names is in it. `ls | cols nmae` used to run
/// `ls`, build the rows, and only then have `tools::unknown_column` scan them — harmless for `ls`,
/// and not harmless at all for a tool a config registered that does something on the way.
///
/// **It may only refuse what it is sure of.** A column set derived from data is
/// [`Columns::Unknown`](crate::data::columns::Columns::Unknown) and refuses nothing; an operand that
/// is not a plain literal is not read, by the same rule [`simple_command_name`] follows. Everything
/// this cannot see is still caught by `unknown_column` when the rows exist.
pub(super) fn refuse_unknown_column(pipeline: &Pipeline) -> Option<String> {
    use crate::data::columns::{Columns, through};
    let mut columns = Columns::Unknown;
    for command in &pipeline.commands {
        let Command::Simple(simple) = command else {
            columns = Columns::Unknown;
            continue;
        };
        let Some(name) = simple_command_name(simple) else {
            columns = Columns::Unknown;
            continue;
        };
        if crate::data::tool::lookup(&name).is_none() {
            // An external in the middle: whatever it prints, nothing here knows its columns.
            columns = Columns::Unknown;
            continue;
        }
        // A word that comes out of an expansion is not known until it runs, so it is not judged.
        let Some(words) = literal_words(simple) else {
            columns = Columns::Unknown;
            continue;
        };
        for wanted in column_operands(&name, &words) {
            if !columns.accepts(wanted) {
                return Some(format!("{name}: {wanted}: no such column"));
            }
        }
        columns = through(&name, &words, &columns);
    }
    None
}

/// Every word of a simple command as a plain literal, or `None` if any of them is not.
///
/// All or nothing: a command with one expanded word has operands at unknown positions, so reading
/// the rest of them would be reading the wrong ones.
fn literal_words(simple: &SimpleCommand) -> Option<Vec<String>> {
    simple
        .words
        .iter()
        .map(|word| match word.parts.as_slice() {
            [WordPart::Literal(text)] => Some(text.clone()),
            _ => None,
        })
        .collect()
}

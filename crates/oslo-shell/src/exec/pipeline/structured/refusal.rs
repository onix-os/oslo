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

/// A structured verb in the middle of a pipeline that redirects its own stdout.
///
/// **What used to happen was the worst of the three possibilities.** The planner forces text on such
/// a stage, which leaves no structured edge, so the whole line fell to the byte path — where the
/// verbs are not commands at all. `ls | first 2 > mid.txt | cat` answered `first: command not found`
/// and *created an empty `mid.txt`* on the way, because the byte path applies a redirection before
/// discovering there is nothing to run. A diagnostic naming the wrong problem, and a side effect for
/// a pipeline that never ran.
///
/// Refused here instead, before anything is applied. Only a *middle* stage: a redirection on the
/// last one is the ordinary case and `run` applies it. And only stdout — `2>/dev/null` leaves the
/// rows alone and is applied around its own stage.
///
/// Rows cross in memory rather than on a descriptor, so a verb whose output went to a file would
/// leave the next stage with nothing to read. Saying so is better than either half-doing it or
/// pretending the name does not exist.
pub(crate) fn refuse_redirected_middle(pipeline: &Pipeline) -> Option<String> {
    let last = pipeline.commands.len().checked_sub(1)?;
    for command in &pipeline.commands[..last] {
        let Command::Simple(simple) = command else {
            continue;
        };
        let Some(name) = super::simple_command_name(simple) else {
            continue;
        };
        if crate::data::tool::lookup(&name).is_none() {
            continue;
        }
        if simple.redirections.iter().any(super::redirects_stdout) {
            return Some(format!(
                "{name}: a verb in the middle of a pipeline cannot redirect its output: \
                 its rows go to the next stage, which would then have nothing to read. \
                 Redirect the last stage instead."
            ));
        }
    }
    None
}

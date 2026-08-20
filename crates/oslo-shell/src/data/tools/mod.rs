//! The commands that understand structure.
//!
//! Each one declares what it takes and what it gives; the planner reads those declarations and
//! nothing else. See `docs/features/structured-pipelines.md`.

pub mod bridge;
pub mod df;
pub mod summarise;
pub mod system;
pub mod units;
pub mod verbs;
pub mod where_;

use crate::data::Record;
use crate::data::plan::Shape;
use crate::env::origin_now;

/// What one verb answers: a status, and the rows it passes on.
type Outcome = (i32, Option<Vec<Record>>);

/// Refuse an operand the verb was never going to read.
///
/// **A word a verb ignores is a mistake, not a decoration.** `ls | length extra` answered as
/// though `extra` had not been typed, and `ls | first 5 10` quietly used the 5 — the same
/// silent-acceptance bug that `printf -Z` and `trap -z EXIT` had, in the tools oslo invented
/// rather than the ones it inherited. `wanted` is how many operands the verb actually reads.
fn too_many(name: &str, words: &[String], wanted: usize) -> Option<Outcome> {
    let extra = words.get(wanted + 1)?;
    eprintln!("{}{name}: {extra}: too many arguments", origin_now());
    Some((2, None))
}

/// A count operand: `first 5`, `final 3`. Absent means one.
///
/// A word that is not a whole number used to become 1, so `first -5` and `first many` both
/// answered a single row and looked as though they had worked.
fn count_operand(name: &str, words: &[String]) -> Result<usize, Outcome> {
    match words.get(1) {
        None => Ok(1),
        Some(word) => word.parse::<usize>().map_err(|_| {
            eprintln!("{}{name}: {word}: a count is a whole number", origin_now());
            (2, None)
        }),
    }
}

/// Refuse a column name that no row in the stream has.
///
/// **Not the same as the per-row rule.** Rows are allowed to disagree about their columns, so
/// `cols` keeps a name that only some of them carry — see [`verbs::cols`]. A name that *no* row
/// has is a different thing: it cannot be a legitimate gap, only a typo, and answering with a
/// stream of empty rows is the worst way to report one.
fn unknown_column(name: &str, rows: &[Record], wanted: &[String]) -> Option<Outcome> {
    // Nothing to check against: an empty stream says nothing about which columns exist.
    if rows.is_empty() {
        return None;
    }
    let missing = wanted
        .iter()
        .find(|column| !rows.iter().any(|row| row.get(column).is_some()))?;
    eprintln!("{}{name}: {missing}: no such column", origin_now());
    Some((2, None))
}

/// Declare every structured tool. Called once, at startup.
///
/// This is the *whole* vocabulary that can carry structure. Every name here is one oslo invented,
/// which is what makes the POSIX guarantee mechanical: a script written before oslo existed cannot
/// name any of them, so no edge of it can ever be planned as rows.
pub fn register_all() {
    crate::data::tool::register("df", Shape::Nothing, Shape::Rows);
    crate::data::tool::register("ps", Shape::Nothing, Shape::Rows);
    crate::data::tool::register("ls", Shape::Nothing, Shape::Rows);
    crate::data::tool::register("where", Shape::Rows, Shape::Rows);
    // The bridge into structure. These take *bytes* — which is what an external command produces —
    // and manufacture rows, so they work with every program already installed.
    crate::data::tool::register("lines", Shape::Bytes, Shape::Rows);
    crate::data::tool::register("parse", Shape::Bytes, Shape::Rows);
    crate::data::tool::register("from", Shape::Bytes, Shape::Rows);
    // The verbs. `cols` rather than `select`, which the parser refuses as a bash keyword.
    for name in ["cols", "get", "sort-by", "first", "final", "length", "each"] {
        crate::data::tool::register(name, Shape::Rows, Shape::Rows);
    }
    // The verbs that make a stream smaller. See `summarise` for why these four and not `join`.
    for name in ["group-by", "count", "distinct", "stats"] {
        crate::data::tool::register(name, Shape::Rows, Shape::Rows);
    }
    // The way out. Rows in, bytes out — so `... | to json | jq .` works, and the structured world
    // is not a place you cannot leave.
    crate::data::tool::register("to", Shape::Rows, Shape::Bytes);
}

/// Run one structured stage, or `None` if the name is not a tool that can run here.
///
/// Answers the exit status and the rows it produced.
pub fn run_tool(
    name: &str,
    words: &[String],
    input: Option<Vec<Record>>,
    bytes: Option<&str>,
) -> Option<(i32, Option<Vec<Record>>)> {
    // A tool the config registered. Looked up first so a config can add a name the shell does not
    // know; it cannot replace one it does, because a name already registered keeps its meaning.
    //
    // **The rows that reached this stage are handed over.** They used to be dropped here, which is
    // what made every Lua tool a source: `notes` was expressible and `redact` was not. The planner
    // was already deciding the edge from `accepts`; this is the other half of that decision.
    if let Some(outcome) = crate::data::custom::rows_of(name, words, input.as_deref()) {
        return match outcome {
            Ok(rows) => Some((0, Some(rows))),
            Err(e) => {
                eprintln!("{}{e}", origin_now());
                Some((1, None))
            }
        };
    }
    match name {
        "ps" => Some((0, Some(system::ps()))),
        // Status 2, which is what the ordinary `ls` answers a path it cannot read — the structured
        // one is the same command wearing a different coat, and the two must not disagree.
        "ls" => match system::ls(&words[1..]) {
            Ok(rows) => Some((0, Some(rows))),
            Err(e) => {
                eprintln!("{}{e}", origin_now());
                Some((2, None))
            }
        },
        "lines" => Some((0, Some(bridge::lines(bytes.unwrap_or_default())))),
        "parse" => {
            let Some(pattern) = words.get(1) else {
                eprintln!(
                    "{}parse: a pattern is required, as in parse '{{user}}:{{uid}}'",
                    origin_now()
                );
                return Some((2, None));
            };
            match bridge::parse(bytes.unwrap_or_default(), pattern) {
                Ok(rows) => Some((0, Some(rows))),
                Err(e) => {
                    eprintln!("{}{e}", origin_now());
                    Some((2, None))
                }
            }
        }
        "from" => {
            // `from json` rather than `from-json`: the format is an argument, so a format oslo
            // learns later needs no new command name.
            match words.get(1).map(String::as_str) {
                Some("json") => match bridge::from_json(bytes.unwrap_or_default()) {
                    Ok(rows) => Some((0, Some(rows))),
                    Err(e) => {
                        eprintln!("{}{e}", origin_now());
                        Some((1, None))
                    }
                },
                Some(other) => {
                    eprintln!(
                        "{}from: {other}: unknown format; oslo knows json",
                        origin_now()
                    );
                    Some((2, None))
                }
                None => {
                    eprintln!(
                        "{}from: a format is required, as in `from json`",
                        origin_now()
                    );
                    Some((2, None))
                }
            }
        }
        "df" => match df::rows() {
            Ok(rows) => Some((0, Some(rows))),
            Err(e) => {
                eprintln!("{}{e}", origin_now());
                Some((1, None))
            }
        },
        "cols" => {
            let names: Vec<String> = words[1..].to_vec();
            if names.is_empty() {
                eprintln!("{}cols: name at least one column", origin_now());
                return Some((2, None));
            }
            let rows = input.unwrap_or_default();
            if let Some(bad) = unknown_column(name, &rows, &names) {
                return Some(bad);
            }
            Some((0, Some(verbs::cols(&rows, &names))))
        }
        "get" | "sort-by" | "group-by" | "stats" => {
            let Some(column) = words.get(1) else {
                eprintln!("{}{name}: a column name is required", origin_now());
                return Some((2, None));
            };
            if let Some(bad) = too_many(name, words, 1) {
                return Some(bad);
            }
            let rows = input.unwrap_or_default();
            let wanted = [column.clone()];
            if let Some(bad) = unknown_column(name, &rows, &wanted) {
                return Some(bad);
            }
            Some((
                0,
                Some(match name {
                    "get" => verbs::get(&rows, column),
                    "sort-by" => verbs::sort_by(&rows, column),
                    "group-by" => summarise::group_by(&rows, column),
                    _ => summarise::stats(&rows, column),
                }),
            ))
        }
        "first" | "final" => {
            if let Some(bad) = too_many(name, words, 1) {
                return Some(bad);
            }
            let n = match count_operand(name, words) {
                Ok(n) => n,
                Err(bad) => return Some(bad),
            };
            let rows = input.unwrap_or_default();
            let taken = if name == "first" {
                verbs::first(&rows, n)
            } else {
                verbs::final_rows(&rows, n)
            };
            Some((0, Some(taken)))
        }
        "length" | "count" => {
            if let Some(bad) = too_many(name, words, 0) {
                return Some(bad);
            }
            let rows = input.unwrap_or_default();
            Some((
                0,
                Some(if name == "length" {
                    verbs::length(&rows)
                } else {
                    summarise::count(&rows)
                }),
            ))
        }
        "distinct" => {
            if let Some(bad) = too_many(name, words, 1) {
                return Some(bad);
            }
            let rows = input.unwrap_or_default();
            if let Some(column) = words.get(1)
                && let Some(bad) = unknown_column(name, &rows, std::slice::from_ref(column))
            {
                return Some(bad);
            }
            Some((
                0,
                Some(summarise::distinct(&rows, words.get(1).map(String::as_str))),
            ))
        }
        "each" => {
            let Some(expression) = words.get(1) else {
                eprintln!("{}each: an expression is required", origin_now());
                return Some((2, None));
            };
            if let Some(bad) = too_many(name, words, 1) {
                return Some(bad);
            }
            let rows = input.unwrap_or_default();
            match where_::for_each(&rows, expression) {
                Some(message) => {
                    eprintln!("{}{message}", origin_now());
                    Some((1, None))
                }
                // `each` is the pressure valve, not a filter: it runs the expression for its side
                // effects and produces no rows, so the pipeline ends here.
                None => Some((0, None)),
            }
        }
        "to" => {
            let Some(format) = words.get(1) else {
                eprintln!("{}to: a format is required, as in `to json`", origin_now());
                return Some((2, None));
            };
            if let Some(bad) = too_many(name, words, 1) {
                return Some(bad);
            }
            match verbs::to_format(&input.unwrap_or_default(), format) {
                Ok(text) => {
                    println!("{text}");
                    Some((0, None))
                }
                Err(e) => {
                    eprintln!("{}{e}", origin_now());
                    Some((2, None))
                }
            }
        }
        "where" => {
            let Some(expression) = words.get(1) else {
                eprintln!("{}where: an expression is required", origin_now());
                return Some((2, None));
            };
            if let Some(bad) = too_many(name, words, 1) {
                return Some(bad);
            }
            let rows = input.unwrap_or_default();
            let (kept, failure) = where_::filter(&rows, expression);
            if let Some(message) = failure {
                eprintln!("{}{message}", origin_now());
                return Some((1, Some(kept)));
            }
            Some((0, Some(kept)))
        }
        _ => None,
    }
}

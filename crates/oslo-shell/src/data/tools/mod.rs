//! The commands that understand structure.
//!
//! Each one declares what it takes and what it gives; the planner reads those declarations and
//! nothing else. See `docs/features/structured-pipelines.md`.

pub mod bridge;
pub mod df;
pub mod summarise;
pub mod system;
pub mod verbs;
pub mod where_;

use crate::data::Record;
use crate::data::plan::Shape;

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
                eprintln!("oslo: {e}");
                Some((1, None))
            }
        };
    }
    match name {
        "ps" => Some((0, Some(system::ps()))),
        "ls" => Some((0, Some(system::ls(&words[1..])))),
        "lines" => Some((0, Some(bridge::lines(bytes.unwrap_or_default())))),
        "parse" => {
            let Some(pattern) = words.get(1) else {
                eprintln!("oslo: parse: a pattern is required, as in parse '{{user}}:{{uid}}'");
                return Some((2, None));
            };
            match bridge::parse(bytes.unwrap_or_default(), pattern) {
                Ok(rows) => Some((0, Some(rows))),
                Err(e) => {
                    eprintln!("oslo: {e}");
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
                        eprintln!("oslo: {e}");
                        Some((1, None))
                    }
                },
                Some(other) => {
                    eprintln!("oslo: from: {other}: unknown format; oslo knows json");
                    Some((2, None))
                }
                None => {
                    eprintln!("oslo: from: a format is required, as in `from json`");
                    Some((2, None))
                }
            }
        }
        "df" => match df::rows() {
            Ok(rows) => Some((0, Some(rows))),
            Err(e) => {
                eprintln!("oslo: {e}");
                Some((1, None))
            }
        },
        "cols" => {
            let names: Vec<String> = words[1..].to_vec();
            if names.is_empty() {
                eprintln!("oslo: cols: name at least one column");
                return Some((2, None));
            }
            Some((0, Some(verbs::cols(&input.unwrap_or_default(), &names))))
        }
        "get" => match words.get(1) {
            Some(name) => Some((0, Some(verbs::get(&input.unwrap_or_default(), name)))),
            None => {
                eprintln!("oslo: get: a column name is required");
                Some((2, None))
            }
        },
        "sort-by" => match words.get(1) {
            Some(name) => Some((0, Some(verbs::sort_by(&input.unwrap_or_default(), name)))),
            None => {
                eprintln!("oslo: sort-by: a column name is required");
                Some((2, None))
            }
        },
        "first" | "final" => {
            let n = words
                .get(1)
                .and_then(|w| w.parse::<usize>().ok())
                .unwrap_or(1);
            let rows = input.unwrap_or_default();
            let taken = if name == "first" {
                verbs::first(&rows, n)
            } else {
                verbs::final_rows(&rows, n)
            };
            Some((0, Some(taken)))
        }
        "length" => Some((0, Some(verbs::length(&input.unwrap_or_default())))),
        "group-by" => match words.get(1) {
            Some(name) => Some((
                0,
                Some(summarise::group_by(&input.unwrap_or_default(), name)),
            )),
            None => {
                eprintln!("oslo: group-by: a column name is required");
                Some((2, None))
            }
        },
        "count" => Some((0, Some(summarise::count(&input.unwrap_or_default())))),
        "distinct" => Some((
            0,
            Some(summarise::distinct(
                &input.unwrap_or_default(),
                words.get(1).map(String::as_str),
            )),
        )),
        "stats" => match words.get(1) {
            Some(name) => Some((0, Some(summarise::stats(&input.unwrap_or_default(), name)))),
            None => {
                eprintln!("oslo: stats: a column name is required");
                Some((2, None))
            }
        },
        "each" => {
            let Some(expression) = words.get(1) else {
                eprintln!("oslo: each: an expression is required");
                return Some((2, None));
            };
            let rows = input.unwrap_or_default();
            match where_::for_each(&rows, expression) {
                Some(message) => {
                    eprintln!("oslo: {message}");
                    Some((1, None))
                }
                // `each` is the pressure valve, not a filter: it runs the expression for its side
                // effects and produces no rows, so the pipeline ends here.
                None => Some((0, None)),
            }
        }
        "to" => {
            let Some(format) = words.get(1) else {
                eprintln!("oslo: to: a format is required, as in `to json`");
                return Some((2, None));
            };
            match verbs::to_format(&input.unwrap_or_default(), format) {
                Ok(text) => {
                    println!("{text}");
                    Some((0, None))
                }
                Err(e) => {
                    eprintln!("oslo: {e}");
                    Some((2, None))
                }
            }
        }
        "where" => {
            let Some(expression) = words.get(1) else {
                eprintln!("oslo: where: an expression is required");
                return Some((2, None));
            };
            let rows = input.unwrap_or_default();
            let (kept, failure) = where_::filter(&rows, expression);
            if let Some(message) = failure {
                eprintln!("oslo: {message}");
                return Some((1, Some(kept)));
            }
            Some((0, Some(kept)))
        }
        _ => None,
    }
}

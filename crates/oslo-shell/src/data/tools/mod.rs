//! The commands that understand structure.
//!
//! Each one declares what it takes and what it gives; the planner reads those declarations and
//! nothing else. See `docs/features/structured-pipelines.md`.

pub mod bridge;
/// The four stages that turn bytes into rows — see the module note there.
mod bridges;
pub mod detect;
pub mod df;
pub mod explore;
pub mod formats;
/// Reading a verb's operands, and refusing the ones it cannot honour.
mod operands;
pub mod past;
#[cfg(feature = "text")]
mod path;
/// Declaring every structured name, once, at startup.
mod registry;
pub mod reshape;
/// What `text` and `path` share: where their strings come from.
#[cfg(feature = "text")]
mod scalar;
pub mod second;
pub mod summarise;
pub mod system;
#[cfg(feature = "text")]
pub mod text;
pub mod units;
pub mod verbs;
pub mod where_;
use operands::{count_operand, sort_operands, too_many, unknown_column};
pub use registry::register_all;

use crate::data::Record;
use crate::data::plan::Shape;
use crate::env::origin_now;

/// The help line under an expression that did not run.
///
/// **The row's columns are the globals**, and that is the fact nearly every failure here turns on:
/// somebody wrote `row.size` or `$size` or `"size"`, and the message from Lua is about a nil or a
/// syntax error rather than about the one convention they did not know.
const LUA_ROW: &str =
    "the row's columns are the names in scope: `size > 1000`, not `row.size` or `$size`";

/// What one verb answers: a status, and the rows it passes on.
type Outcome = (i32, Option<Vec<Record>>);

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
    // **The bytes go through too.** `accepts = "bytes"` was a declared shape that could not
    // work: the planner routed the bytes here (see `exec::pipeline::structured`, which reads
    // standard input for a bytes-accepting tool at the head of a pipeline), and this line dropped
    // them one call short of the handler.
    if let Some(outcome) = crate::data::custom::rows_of(name, words, input.as_deref(), bytes) {
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
        // What this shell has already been asked to do. No failure arm: a shell with no store open
        // has an empty past, which is an answer rather than an error.
        "history" => Some((0, Some(past::rows()))),
        // Status 2, which is what the ordinary `ls` answers a path it cannot read — the structured
        // one is the same command wearing a different coat, and the two must not disagree.
        "ls" => match system::ls(&words[1..]) {
            Ok(rows) => Some((0, Some(rows))),
            Err(e) => {
                eprintln!("{}{e}", origin_now());
                Some((2, None))
            }
        },
        "df" => match df::rows() {
            Ok(rows) => Some((0, Some(rows))),
            Err(e) => {
                eprintln!("{}{e}", origin_now());
                Some((1, None))
            }
        },
        "lines" | "parse" | "detect-columns" | "from" => bridges::run(name, words, bytes),
        #[cfg(feature = "text")]
        "text" => text::run(words, input.as_deref(), bytes),
        #[cfg(feature = "text")]
        "path" => path::run(words, input.as_deref(), bytes),
        "cols" => {
            let names: Vec<String> = words[1..].to_vec();
            if names.is_empty() {
                crate::env::complain(
                    words,
                    "cols",
                    "cols: name at least one column",
                    "which columns?",
                    None,
                );
                return Some((2, None));
            }
            let rows = input.unwrap_or_default();
            if let Some(bad) = unknown_column(name, words, &rows, &names) {
                return Some(bad);
            }
            Some((0, Some(verbs::cols(&rows, &names))))
        }
        "sort-by" => {
            let (options, keys) = match sort_operands(&words[1..]) {
                Ok(parsed) => parsed,
                Err(bad) => return Some(bad),
            };
            if keys.is_empty() {
                crate::env::complain(
                    words,
                    "sort-by",
                    "sort-by: a column name is required",
                    "sort by which column?",
                    None,
                );
                return Some((2, None));
            }
            let rows = input.unwrap_or_default();
            if let Some(bad) = unknown_column(name, words, &rows, &keys) {
                return Some(bad);
            }
            Some((0, Some(verbs::sort_by(&rows, &keys, options))))
        }
        "reverse" | "flatten" | "headers" | "enumerate" | "describe" => {
            if let Some(bad) = too_many(name, words, 0) {
                return Some(bad);
            }
            let rows = input.unwrap_or_default();
            Some((
                0,
                Some(match name {
                    "reverse" => verbs::reverse(&rows),
                    "flatten" => reshape::flatten(&rows),
                    "headers" => reshape::headers(&rows),
                    "describe" => summarise::describe(&rows),
                    _ => reshape::enumerate(&rows),
                }),
            ))
        }
        "lookup" | "append" | "merge" => {
            // `--keep` is the left-outer form of `lookup`, and meaningless on the other two.
            let keep = words.get(1).is_some_and(|w| w == "--keep");
            if keep && name != "lookup" {
                crate::env::complain(
                    words,
                    "--keep",
                    &format!("{name}: --keep is a lookup option"),
                    "only lookup takes this",
                    None,
                );
                return Some((2, None));
            }
            let at = if keep { 2 } else { 1 };
            let Some(expression) = words.get(at) else {
                eprintln!(
                    "{}{name}: the other stream is required, as a Lua expression answering rows",
                    origin_now()
                );
                return Some((2, None));
            };
            // `lookup` needs a key; the other two pair by position or by order.
            let key = match name {
                "lookup" => match words.get(at + 1) {
                    Some(key) => Some(key.clone()),
                    None => {
                        crate::env::complain(
                            words,
                            "lookup",
                            "lookup: a column to join on is required",
                            "join on which column?",
                            None,
                        );
                        return Some((2, None));
                    }
                },
                _ => None,
            };
            if let Some(bad) = too_many(name, words, at + usize::from(key.is_some())) {
                return Some(bad);
            }
            let other = match where_::rows_from(expression) {
                Ok(rows) => rows,
                Err(e) => {
                    eprintln!("{}{name}: {e}", origin_now());
                    return Some((1, None));
                }
            };
            let rows = input.unwrap_or_default();
            Some((
                0,
                Some(match name {
                    "lookup" => {
                        let key = key.unwrap_or_default();
                        if let Some(bad) =
                            unknown_column(name, words, &rows, std::slice::from_ref(&key))
                        {
                            return Some(bad);
                        }
                        second::lookup(&rows, &other, &key, keep)
                    }
                    "append" => second::append(&rows, &other),
                    _ => second::merge(&rows, &other),
                }),
            ))
        }
        "histogram" => {
            let Some(column) = words.get(1) else {
                crate::env::complain(
                    words,
                    "histogram",
                    "histogram: a column name is required",
                    "count which column?",
                    None,
                );
                return Some((2, None));
            };
            if let Some(bad) = too_many(name, words, 1) {
                return Some(bad);
            }
            let rows = input.unwrap_or_default();
            if let Some(bad) = unknown_column(name, words, &rows, std::slice::from_ref(column)) {
                return Some(bad);
            }
            Some((0, Some(summarise::histogram(&rows, column))))
        }
        "reduce" => {
            // `--from` before the expression, so the expression is always the last word and a fold
            // that starts from text reads `reduce --from '' 'acc .. name'`.
            let from = words.get(1).filter(|w| *w == "--from").is_some();
            let (start, at) = match from {
                true => (words.get(2).map(String::as_str), 3),
                false => (None, 1),
            };
            let Some(expression) = words.get(at) else {
                crate::env::complain(
                    words,
                    "reduce",
                    "reduce: an expression is required",
                    "reduce with what?",
                    None,
                );
                return Some((2, None));
            };
            if let Some(bad) = too_many(name, words, at) {
                return Some(bad);
            }
            let rows = input.unwrap_or_default();
            let (folded, failure) = where_::reduce(&rows, expression, start);
            if let Some(message) = failure {
                crate::env::complain(
                    words,
                    expression,
                    &message,
                    "this expression",
                    Some(LUA_ROW),
                );
                return Some((1, Some(folded)));
            }
            Some((0, Some(folded)))
        }
        "reject" => {
            let names: Vec<String> = words[1..].to_vec();
            if names.is_empty() {
                crate::env::complain(
                    words,
                    "reject",
                    "reject: name at least one column",
                    "which columns?",
                    None,
                );
                return Some((2, None));
            }
            let rows = input.unwrap_or_default();
            if let Some(bad) = unknown_column(name, words, &rows, &names) {
                return Some(bad);
            }
            Some((0, Some(reshape::reject(&rows, &names))))
        }
        "rename" => {
            let (Some(from), Some(to)) = (words.get(1), words.get(2)) else {
                eprintln!(
                    "{}rename: an old name and a new one are required",
                    origin_now()
                );
                return Some((2, None));
            };
            if let Some(bad) = too_many(name, words, 2) {
                return Some(bad);
            }
            let rows = input.unwrap_or_default();
            if let Some(bad) = unknown_column(name, words, &rows, std::slice::from_ref(from)) {
                return Some(bad);
            }
            Some((0, Some(reshape::rename(&rows, from, to))))
        }
        "insert" | "update" | "upsert" => {
            let (Some(column), Some(expression)) = (words.get(1), words.get(2)) else {
                eprintln!(
                    "{}{name}: a column name and an expression are required",
                    origin_now()
                );
                return Some((2, None));
            };
            if let Some(bad) = too_many(name, words, 2) {
                return Some(bad);
            }
            let rows = input.unwrap_or_default();
            let (values, failure) = where_::compute(&rows, expression);
            if let Some(message) = failure {
                eprintln!("{}{name}: {message}", origin_now());
            }
            let when = match name {
                "insert" => reshape::When::Absent,
                "update" => reshape::When::Present,
                _ => reshape::When::Either,
            };
            match reshape::assign(&rows, column, &values, when) {
                Ok(out) => Some((0, Some(out))),
                Err(e) => {
                    eprintln!("{}{e}", origin_now());
                    Some((2, None))
                }
            }
        }
        "skip" | "every" => {
            if let Some(bad) = too_many(name, words, 1) {
                return Some(bad);
            }
            let n = match count_operand(name, words) {
                Ok(n) => n,
                Err(bad) => return Some(bad),
            };
            let rows = input.unwrap_or_default();
            Some((
                0,
                Some(match name {
                    "skip" => reshape::skip(&rows, n),
                    _ => reshape::every(&rows, n),
                }),
            ))
        }
        "compact" => {
            if let Some(bad) = too_many(name, words, 1) {
                return Some(bad);
            }
            let rows = input.unwrap_or_default();
            if let Some(column) = words.get(1)
                && let Some(bad) = unknown_column(name, words, &rows, std::slice::from_ref(column))
            {
                return Some(bad);
            }
            Some((
                0,
                Some(reshape::compact(&rows, words.get(1).map(String::as_str))),
            ))
        }
        "default" => {
            let (Some(column), Some(value)) = (words.get(1), words.get(2)) else {
                eprintln!(
                    "{}default: a column name and a value are required",
                    origin_now()
                );
                return Some((2, None));
            };
            if let Some(bad) = too_many(name, words, 2) {
                return Some(bad);
            }
            let rows = input.unwrap_or_default();
            // A word off a command line is text; a number is read as one so `default n 0` fills
            // with something `sort-by` and `stats` can do arithmetic on.
            let filled = match value.parse::<i64>() {
                Ok(n) => crate::data::Val::Int(n),
                Err(_) => match value.parse::<f64>() {
                    Ok(f) if value.contains('.') => crate::data::Val::Float(f),
                    _ => crate::data::Val::Str(value.clone()),
                },
            };
            Some((0, Some(reshape::default(&rows, column, &filled))))
        }
        "get" | "group-by" | "stats" => {
            let Some(column) = words.get(1) else {
                crate::env::complain(
                    words,
                    name,
                    &format!("{name}: a column name is required"),
                    "which column?",
                    None,
                );
                return Some((2, None));
            };
            if let Some(bad) = too_many(name, words, 1) {
                return Some(bad);
            }
            let rows = input.unwrap_or_default();
            let wanted = [column.clone()];
            if let Some(bad) = unknown_column(name, words, &rows, &wanted) {
                return Some(bad);
            }
            Some((
                0,
                Some(match name {
                    "get" => verbs::get(&rows, column),
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
                && let Some(bad) = unknown_column(name, words, &rows, std::slice::from_ref(column))
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
                crate::env::complain(
                    words,
                    "each",
                    "each: an expression is required",
                    "do what to each row?",
                    None,
                );
                return Some((2, None));
            };
            if let Some(bad) = too_many(name, words, 1) {
                return Some(bad);
            }
            let rows = input.unwrap_or_default();
            match where_::for_each(&rows, expression) {
                Some(message) => {
                    crate::env::complain(
                        words,
                        expression,
                        &message,
                        "this expression",
                        Some(LUA_ROW),
                    );
                    Some((1, None))
                }
                // `each` is the pressure valve, not a filter: it runs the expression for its side
                // effects and produces no rows, so the pipeline ends here.
                None => Some((0, None)),
            }
        }
        "explore" => {
            if let Some(bad) = too_many(name, words, 0) {
                return Some(bad);
            }
            let rows = input.unwrap_or_default();
            let sheet = explore::sheet(name, &rows);
            // Rows are never handed on. A viewer that also passed its input through would make
            // `ps | explore | length` block on a person and then answer a number, which is two
            // things at once; `explore` is the end of the line, like `each`.
            match oslo_ui::explore::open(sheet) {
                oslo_ui::explore::Outcome::Closed => Some((0, None)),
                // Neither is a failure of the pipeline — the rows were computed, there was just
                // nothing to look at or nowhere to look at it. Said out loud, because a viewer
                // that opened and closed instantly with no word about why is the worst of both.
                oslo_ui::explore::Outcome::Empty => {
                    eprintln!("{}explore: no rows", origin_now());
                    Some((0, None))
                }
                oslo_ui::explore::Outcome::NoTerminal => {
                    eprintln!("{}explore: no terminal to draw on", origin_now());
                    Some((1, None))
                }
            }
        }
        "to" => {
            let Some(format) = words.get(1) else {
                crate::env::complain(
                    words,
                    "to",
                    "to: a format is required, as in `to json`",
                    "which format?",
                    None,
                );
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
        "where" | "map" => {
            let Some(expression) = words.get(1) else {
                crate::env::complain(
                    words,
                    name,
                    &format!("{name}: an expression is required"),
                    "an expression, in Lua",
                    None,
                );
                return Some((2, None));
            };
            if let Some(bad) = too_many(name, words, 1) {
                return Some(bad);
            }
            let rows = input.unwrap_or_default();
            let (kept, failure) = match name {
                "where" => where_::filter(&rows, expression),
                _ => where_::map_rows(&rows, expression),
            };
            if let Some(message) = failure {
                crate::env::complain(
                    words,
                    expression,
                    &message,
                    "this expression",
                    Some(LUA_ROW),
                );
                return Some((1, Some(kept)));
            }
            Some((0, Some(kept)))
        }
        _ => None,
    }
}

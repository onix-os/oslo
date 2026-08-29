//! The commands that understand structure.
//!
//! Each one declares what it takes and what it gives; the planner reads those declarations and
//! nothing else. See `docs/features/structured-pipelines.md`.

pub mod bridge;
pub mod detect;
pub mod df;
pub mod formats;
pub mod reshape;
pub mod second;
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

/// The flags and keys of a `sort-by`.
///
/// Short flags cluster (`-rn`) the way every shell user expects, and `--` ends them — a column
/// really could be called `-x`, and POSIX has one way of saying so.
fn sort_operands(words: &[String]) -> Result<(verbs::SortOptions, Vec<String>), Outcome> {
    let mut options = verbs::SortOptions::default();
    let mut keys = Vec::new();
    let mut flags_done = false;
    for word in words {
        if flags_done || !word.starts_with('-') || word == "-" {
            keys.push(word.clone());
            continue;
        }
        if word == "--" {
            flags_done = true;
            continue;
        }
        let long = word.strip_prefix("--");
        let ok = match long {
            Some("reverse") => set(&mut options.reverse),
            Some("natural") => set(&mut options.natural),
            Some("ignore-case") => set(&mut options.ignore_case),
            Some(_) => false,
            // A cluster: every letter has to be one this knows, or the whole word is refused.
            None => word.chars().skip(1).all(|c| match c {
                'r' => set(&mut options.reverse),
                'n' => set(&mut options.natural),
                'i' => set(&mut options.ignore_case),
                _ => false,
            }),
        };
        if !ok {
            eprintln!(
                "{}sort-by: {word}: not an option; sort-by knows -r, -n and -i",
                origin_now()
            );
            return Err((2, None));
        }
    }
    Ok((options, keys))
}

fn set(flag: &mut bool) -> bool {
    *flag = true;
    true
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
    // A path counts as present when it resolves in *any* row, and an optional step (`a.b?`) is
    // present by construction — it said the absence was expected, so refusing it here would make
    // `?` mean nothing.
    let missing = wanted.iter().find(|column| {
        let path = crate::data::path::Path::parse(column);
        !rows
            .iter()
            .any(|row| matches!(path.get(row), Ok(Some(_)) | Ok(None)))
    })?;
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
    // Somebody else's aligned output, with no pattern to write and nothing for them to agree to.
    crate::data::tool::register("detect-columns", Shape::Bytes, Shape::Rows);
    // The verbs. `cols` rather than `select`, which the parser refuses as a bash keyword.
    // `map` answers a row per row; `each` answers none and ends the pipeline. Two names because
    // they are two things — a flag on one would make "does this produce rows" a runtime question,
    // and the planner has to know it before anything runs.
    for name in [
        "cols", "get", "sort-by", "first", "final", "length", "each", "map", "reverse",
    ] {
        crate::data::tool::register(name, Shape::Rows, Shape::Rows);
    }
    // The verbs that make a stream smaller. See `summarise` for why these and not `join`.
    for name in [
        "group-by",
        "count",
        "distinct",
        "stats",
        "describe",
        "histogram",
        "reduce",
    ] {
        crate::data::tool::register(name, Shape::Rows, Shape::Rows);
    }
    // Reshaping: which columns a stream has, and which rows. See `reshape` for the twelve taken and
    // the ten deliberately not, which is a decision about the name budget rather than about effort.
    for name in [
        "reject",
        "rename",
        "insert",
        "update",
        "upsert",
        "flatten",
        "headers",
        "skip",
        "every",
        "enumerate",
        "compact",
        "default",
    ] {
        crate::data::tool::register(name, Shape::Rows, Shape::Rows);
    }
    // The verbs that need a second stream, which they name as a Lua expression because the pipeline
    // is a line. `lookup` rather than `join`, which is POSIX. See `second`.
    for name in ["lookup", "append", "merge"] {
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
            // `--regex` swaps the pattern language, not the verb: one name, two ways of saying what
            // the columns are.
            let by_regex = words.get(1).is_some_and(|w| w == "--regex");
            let at = if by_regex { 2 } else { 1 };
            let Some(pattern) = words.get(at) else {
                eprintln!(
                    "{}parse: a pattern is required, as in parse '{{user}}:{{uid}}'",
                    origin_now()
                );
                return Some((2, None));
            };
            if let Some(bad) = too_many(name, words, at) {
                return Some(bad);
            }
            let read = match by_regex {
                true => bridge::parse_regex(bytes.unwrap_or_default(), pattern),
                false => bridge::parse(bytes.unwrap_or_default(), pattern),
            };
            match read {
                Ok(rows) => Some((0, Some(rows))),
                Err(e) => {
                    eprintln!("{}{e}", origin_now());
                    Some((2, None))
                }
            }
        }
        "detect-columns" => {
            let mut layout = detect::Layout::default();
            let mut rest = words[1..].iter();
            while let Some(word) = rest.next() {
                match word.as_str() {
                    "--no-headers" => layout.no_headers = true,
                    "--skip" => match rest.next().and_then(|n| n.parse::<usize>().ok()) {
                        Some(n) => layout.skip = n,
                        None => {
                            eprintln!(
                                "{}detect-columns: --skip takes a whole number of lines",
                                origin_now()
                            );
                            return Some((2, None));
                        }
                    },
                    other => {
                        eprintln!(
                            "{}detect-columns: {other}: not an option; it knows --no-headers and --skip",
                            origin_now()
                        );
                        return Some((2, None));
                    }
                }
            }
            Some((0, Some(detect::detect(bytes.unwrap_or_default(), layout))))
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
                Some(format) if formats::delimiter(format).is_some() => {
                    let delimiter = formats::delimiter(format).unwrap_or(',');
                    match formats::from_delimited(bytes.unwrap_or_default(), delimiter) {
                        Ok(rows) => Some((0, Some(rows))),
                        Err(e) => {
                            eprintln!("{}from {format}: {e}", origin_now());
                            Some((1, None))
                        }
                    }
                }
                Some(other) => {
                    eprintln!(
                        "{}from: {other}: unknown format; oslo knows json, csv and tsv",
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
        "sort-by" => {
            let (options, keys) = match sort_operands(&words[1..]) {
                Ok(parsed) => parsed,
                Err(bad) => return Some(bad),
            };
            if keys.is_empty() {
                eprintln!("{}sort-by: a column name is required", origin_now());
                return Some((2, None));
            }
            let rows = input.unwrap_or_default();
            if let Some(bad) = unknown_column(name, &rows, &keys) {
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
                eprintln!("{}{name}: --keep is a lookup option", origin_now());
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
                        eprintln!("{}lookup: a column to join on is required", origin_now());
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
                        if let Some(bad) = unknown_column(name, &rows, std::slice::from_ref(&key)) {
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
                eprintln!("{}histogram: a column name is required", origin_now());
                return Some((2, None));
            };
            if let Some(bad) = too_many(name, words, 1) {
                return Some(bad);
            }
            let rows = input.unwrap_or_default();
            if let Some(bad) = unknown_column(name, &rows, std::slice::from_ref(column)) {
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
                eprintln!("{}reduce: an expression is required", origin_now());
                return Some((2, None));
            };
            if let Some(bad) = too_many(name, words, at) {
                return Some(bad);
            }
            let rows = input.unwrap_or_default();
            let (folded, failure) = where_::reduce(&rows, expression, start);
            if let Some(message) = failure {
                eprintln!("{}{message}", origin_now());
                return Some((1, Some(folded)));
            }
            Some((0, Some(folded)))
        }
        "reject" => {
            let names: Vec<String> = words[1..].to_vec();
            if names.is_empty() {
                eprintln!("{}reject: name at least one column", origin_now());
                return Some((2, None));
            }
            let rows = input.unwrap_or_default();
            if let Some(bad) = unknown_column(name, &rows, &names) {
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
            if let Some(bad) = unknown_column(name, &rows, std::slice::from_ref(from)) {
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
                && let Some(bad) = unknown_column(name, &rows, std::slice::from_ref(column))
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
        "where" | "map" => {
            let Some(expression) = words.get(1) else {
                eprintln!("{}{name}: an expression is required", origin_now());
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
                eprintln!("{}{message}", origin_now());
                return Some((1, Some(kept)));
            }
            Some((0, Some(kept)))
        }
        _ => None,
    }
}

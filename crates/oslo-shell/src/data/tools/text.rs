//! `text` — the things you do to a string, as a verb in the pipeline.
//!
//! ```sh
//! text split : "$PATH" | where 'text:match("local")'
//! ls | text upper
//! text join , a b c
//! ```
//!
//! # Why it is a verb and not a builtin
//!
//! **`text split` makes several values out of one, and in oslo several values are rows.** That is
//! the whole difference from doing this with `cut` or `sed`: the fields carry structure into the
//! next stage instead of being flattened to a line the next stage has to take apart again. A shell
//! without a structured pipeline cannot make that choice, which is why fish's `string` hands back
//! lines and stops there.
//!
//! # Where the strings come from
//!
//! Three sources, in one order, and the first that has anything wins:
//!
//! 1. **the operands** — `text upper a b c`;
//! 2. **the rows** that reached this stage, one string per row;
//! 3. **the bytes**, one string per line.
//!
//! Operands first is what makes `text split : "$PATH"` work at the head of a pipeline, and rows
//! before bytes is what makes `ls | text upper` operate on the column rather than on a rendering
//! of the table.
//!
//! # What comes out
//!
//! Always rows of one column, named `text`. Even `join`, which produces a single value: one row of
//! one column is what a single value *is* here, and `to text` renders it as the bare string — so
//! `x=$(text join , a b)` is the obvious thing. A shape that changed with the subcommand would have
//! to be declared per subcommand, and the registry is keyed by the first word.

use super::super::value::{Record, Val};
use crate::env::origin_now;

/// The column every subcommand answers in.
const COLUMN: &str = "text";

/// Which of `text`'s subcommands take their own operand before the strings begin.
///
/// `text split : a b` — the `:` is the separator and `a b` are the strings. Without knowing this,
/// the separator would be read as one of the strings to split.
fn leading_operands(sub: &str) -> usize {
    match sub {
        "split" | "join" | "match" => 1,
        "replace" => 2,
        _ => 0,
    }
}

/// Run `text`, or say why not.
pub fn run(
    words: &[String],
    input: Option<&[Record]>,
    bytes: Option<&str>,
) -> Option<(i32, Option<Vec<Record>>)> {
    let Some(sub) = words.get(1) else {
        eprintln!(
            "{}text: name a subcommand, as in text split : \"$PATH\"",
            origin_now()
        );
        eprintln!("{}", usage());
        return Some((2, None));
    };

    let (flags, operands) = match split_flags(&words[2..]) {
        Ok(parsed) => parsed,
        Err(problem) => {
            eprintln!("{}text {sub}: {problem}", origin_now());
            return Some((2, None));
        }
    };

    let leading = leading_operands(sub);
    if operands.len() < leading {
        eprintln!(
            "{}text {sub}: needs {leading} more operand(s)",
            origin_now()
        );
        eprintln!("{}", usage());
        return Some((2, None));
    }
    let own = &operands[..leading];
    let strings = gather(&operands[leading..], input, bytes);

    let outcome = match sub.as_str() {
        "split" => split(&own[0], &strings, &flags),
        "join" => join(&own[0], &strings),
        "match" => matching(&own[0], &strings, &flags),
        "replace" => replace(&own[0], &own[1], &strings, &flags),
        "trim" => trim(&strings, &flags),
        "upper" => Ok(each(&strings, |s| s.to_uppercase())),
        "lower" => Ok(each(&strings, |s| s.to_lowercase())),
        "length" => Ok(strings
            .iter()
            .map(|s| row(Val::Int(s.chars().count() as i64)))
            .collect()),
        "sub" => sub_string(&strings, &flags),
        "pad" => pad(&strings, &flags),
        "repeat" => repeat(&strings, &flags),
        "escape" => Ok(each(&strings, escape_one)),
        "unescape" => Ok(each(&strings, unescape_one)),
        "collect" => Ok(vec![row(Val::Str(strings.join("\n")))]),
        other => {
            crate::env::complain(
                words,
                other,
                &format!("text: {other}: not a subcommand"),
                "no subcommand of that name",
                Some("`text` on its own lists them"),
            );
            eprintln!("{}", usage());
            return Some((2, None));
        }
    };

    match outcome {
        Ok(rows) => Some((0, Some(rows))),
        Err(problem) => {
            eprintln!("{}text {sub}: {problem}", origin_now());
            Some((2, None))
        }
    }
}

/// One row of one column.
fn row(value: Val) -> Record {
    Record::from_pairs([(COLUMN, value)])
}

fn each(strings: &[String], how: impl Fn(&str) -> String) -> Vec<Record> {
    strings.iter().map(|s| row(Val::Str(how(s)))).collect()
}

/// Where a row's string is looked for: `text` first, then `line`, so a stage after `text` and a
/// stage after `lines` both work without naming a column.
const READS: [&str; 2] = [COLUMN, "line"];

fn gather(operands: &[String], input: Option<&[Record]>, bytes: Option<&str>) -> Vec<String> {
    super::scalar::gather(operands, input, bytes, &READS)
}

/// Only the tests read a row back; `gather` does it for everything else.
#[cfg(test)]
fn string_of(record: &Record) -> String {
    super::scalar::string_of(record, &READS)
}

/// Quote a string so a shell reads it back as exactly these characters.
///
/// Single quotes, because they are the only quoting with no interior rules at all. The one case to
/// handle is a single quote itself, which ends the run and has to be re-entered around it.
fn escape_one(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

/// Options, separated from operands.
///
/// `--` ends them, so a separator that looks like a flag can still be given: `text split -- --`.
fn split_flags(words: &[String]) -> std::result::Result<(Flags, Vec<String>), String> {
    let mut flags = Flags::default();
    let mut operands = Vec::new();
    let mut at = 0;
    while at < words.len() {
        let word = &words[at];
        match word.as_str() {
            "--" => {
                operands.extend(words[at + 1..].iter().cloned());
                break;
            }
            "--regex" | "-r" => flags.regex = true,
            "--all" | "-a" => flags.all = true,
            "--invert" | "-v" => flags.invert = true,
            "--ignore-case" | "-i" => flags.ignore_case = true,
            "--left" => flags.left = true,
            "--right" => flags.right = true,
            "--max" | "--count" | "--width" | "--start" | "--length" => {
                let Some(value) = words.get(at + 1) else {
                    return Err(format!("{word}: needs a number"));
                };
                let number: i64 = value
                    .parse()
                    .map_err(|_| format!("{word}: {value}: not a number"))?;
                match word.as_str() {
                    "--max" => flags.max = Some(number),
                    "--count" => flags.count = Some(number),
                    "--width" => flags.width = Some(number),
                    "--start" => flags.start = Some(number),
                    _ => flags.length = Some(number),
                }
                at += 1;
            }
            "--chars" | "--char" => {
                let Some(value) = words.get(at + 1) else {
                    return Err(format!("{word}: needs characters"));
                };
                flags.chars = Some(value.clone());
                at += 1;
            }
            other if other.starts_with('-') && other.len() > 1 => {
                return Err(format!("{other}: not an option"));
            }
            other => operands.push(other.to_string()),
        }
        at += 1;
    }
    Ok((flags, operands))
}

#[derive(Default)]
struct Flags {
    regex: bool,
    all: bool,
    invert: bool,
    ignore_case: bool,
    left: bool,
    right: bool,
    max: Option<i64>,
    count: Option<i64>,
    width: Option<i64>,
    start: Option<i64>,
    length: Option<i64>,
    chars: Option<String>,
}

/// `text split SEP [STRING...]` — a row per field.
///
/// **An empty separator splits into characters**, which is the only thing it could usefully mean
/// and saves a `--chars` nobody would find.
fn split(
    separator: &str,
    strings: &[String],
    flags: &Flags,
) -> std::result::Result<Vec<Record>, String> {
    let mut rows = Vec::new();
    for string in strings {
        if separator.is_empty() {
            rows.extend(string.chars().map(|c| row(Val::Str(c.to_string()))));
            continue;
        }
        let fields: Vec<&str> = match flags.max {
            // `--max n` gives at most n+1 fields, so the last one keeps the rest of the string —
            // which is what makes `text split --max 1 = "a=b=c"` a key and its value.
            Some(most) if most >= 0 => string.splitn(most as usize + 1, separator).collect(),
            _ => string.split(separator).collect(),
        };
        rows.extend(fields.into_iter().map(|f| row(Val::Str(f.to_string()))));
    }
    Ok(rows)
}

/// `text join SEP [STRING...]` — one row.
fn join(separator: &str, strings: &[String]) -> std::result::Result<Vec<Record>, String> {
    Ok(vec![row(Val::Str(strings.join(separator)))])
}

/// `text match PATTERN [STRING...]` — the strings that match, like `grep`.
///
/// A literal by default and a regular expression with `--regex`, because most matching is for a
/// substring and a pattern language nobody asked for is a pattern language that surprises.
fn matching(
    pattern: &str,
    strings: &[String],
    flags: &Flags,
) -> std::result::Result<Vec<Record>, String> {
    let matches: Box<dyn Fn(&str) -> bool> = if flags.regex {
        let compiled = regex::RegexBuilder::new(pattern)
            .case_insensitive(flags.ignore_case)
            .build()
            .map_err(|problem| format!("{pattern}: {problem}"))?;
        Box::new(move |s: &str| compiled.is_match(s))
    } else if flags.ignore_case {
        let needle = pattern.to_lowercase();
        Box::new(move |s: &str| s.to_lowercase().contains(&needle))
    } else {
        let needle = pattern.to_string();
        Box::new(move |s: &str| s.contains(&needle))
    };
    Ok(strings
        .iter()
        .filter(|s| matches(s) != flags.invert)
        .map(|s| row(Val::Str(s.clone())))
        .collect())
}

/// `text replace PATTERN REPLACEMENT [STRING...]`.
///
/// The first occurrence unless `--all`, matching what `${x/a/b}` does — two spellings of one idea
/// should not disagree about how many they change.
fn replace(
    pattern: &str,
    with: &str,
    strings: &[String],
    flags: &Flags,
) -> std::result::Result<Vec<Record>, String> {
    if flags.regex {
        let compiled = regex::RegexBuilder::new(pattern)
            .case_insensitive(flags.ignore_case)
            .build()
            .map_err(|problem| format!("{pattern}: {problem}"))?;
        return Ok(each(strings, |s| match flags.all {
            true => compiled.replace_all(s, with).into_owned(),
            false => compiled.replace(s, with).into_owned(),
        }));
    }
    Ok(each(strings, |s| match flags.all {
        true => s.replace(pattern, with),
        false => s.replacen(pattern, with, 1),
    }))
}

/// `text trim [--left|--right] [--chars SET]`.
///
/// Whitespace unless a set is given, and both ends unless one is named.
fn trim(strings: &[String], flags: &Flags) -> std::result::Result<Vec<Record>, String> {
    let both = !flags.left && !flags.right;
    let set: Option<Vec<char>> = flags.chars.as_ref().map(|c| c.chars().collect());
    Ok(each(strings, |s| {
        let mut out = s;
        if flags.left || both {
            out = match &set {
                Some(chars) => out.trim_start_matches(|c| chars.contains(&c)),
                None => out.trim_start(),
            };
        }
        if flags.right || both {
            out = match &set {
                Some(chars) => out.trim_end_matches(|c| chars.contains(&c)),
                None => out.trim_end(),
            };
        }
        out.to_string()
    }))
}

/// `text sub --start N [--length N]`.
///
/// **Counted in characters and from 1**, like everything else a shell counts. A negative start
/// counts from the end, which is the thing `${x: -3}` makes awkward and people want anyway.
fn sub_string(strings: &[String], flags: &Flags) -> std::result::Result<Vec<Record>, String> {
    let start = flags.start.unwrap_or(1);
    if start == 0 {
        return Err("--start counts from 1; 0 names nothing".to_string());
    }
    Ok(each(strings, |s| {
        let chars: Vec<char> = s.chars().collect();
        let from = match start < 0 {
            true => chars.len().saturating_sub(start.unsigned_abs() as usize),
            false => (start as usize - 1).min(chars.len()),
        };
        let take = match flags.length {
            Some(length) if length >= 0 => length as usize,
            _ => chars.len(),
        };
        chars[from..].iter().take(take).collect()
    }))
}

/// `text pad --width N [--right] [--char C]`.
///
/// Padded on the left unless `--right`, so numbers line up by default. A string already at least
/// that wide is left alone rather than cut — padding and truncating are different requests.
fn pad(strings: &[String], flags: &Flags) -> std::result::Result<Vec<Record>, String> {
    let Some(width) = flags.width else {
        return Err("--width is required".to_string());
    };
    let width = width.max(0) as usize;
    let filler = flags
        .chars
        .as_ref()
        .and_then(|c| c.chars().next())
        .unwrap_or(' ');
    Ok(each(strings, |s| {
        let have = s.chars().count();
        if have >= width {
            return s.to_string();
        }
        let padding: String = std::iter::repeat_n(filler, width - have).collect();
        match flags.right {
            true => format!("{s}{padding}"),
            false => format!("{padding}{s}"),
        }
    }))
}

/// `text repeat --count N`.
fn repeat(strings: &[String], flags: &Flags) -> std::result::Result<Vec<Record>, String> {
    let Some(count) = flags.count else {
        return Err("--count is required".to_string());
    };
    if count < 0 {
        return Err("--count cannot be negative".to_string());
    }
    Ok(each(strings, |s| s.repeat(count as usize)))
}

/// The inverse of `escape`: take one level of shell quoting off.
///
/// Deliberately textual rather than a parse — it undoes what `escape` did, and anything it cannot
/// recognise it leaves exactly as it found it.
fn unescape_one(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.len() >= 2 && trimmed.starts_with('\'') && trimmed.ends_with('\'') {
        return trimmed[1..trimmed.len() - 1].replace("'\\''", "'");
    }
    if trimmed.len() >= 2 && trimmed.starts_with('"') && trimmed.ends_with('"') {
        return trimmed[1..trimmed.len() - 1].replace("\\\"", "\"");
    }
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => out.extend(chars.next()),
            other => out.push(other),
        }
    }
    out
}

fn usage() -> String {
    "usage: text SUBCOMMAND [options] [strings…]\n\
     \n\
     \x20 split SEP [--max N]        a row per field; an empty SEP splits into characters\n\
     \x20 join SEP                   one row, the strings joined\n\
     \x20 match PAT [--regex] [-v]   the strings that match\n\
     \x20 replace PAT NEW [--regex] [--all]\n\
     \x20 trim [--left|--right] [--chars SET]\n\
     \x20 sub --start N [--length N] counted in characters, from 1; a negative start counts back\n\
     \x20 pad --width N [--right] [--char C]\n\
     \x20 repeat --count N\n\
     \x20 upper | lower | length | escape | unescape | collect\n\
     \n\
     The strings are the operands, or the rows, or the input lines — the first of those there is."
        .to_string()
}

#[cfg(test)]
mod tests;

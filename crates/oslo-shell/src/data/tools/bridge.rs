//! Turning bytes into rows: `lines`, `parse` and `from json`.
//!
//! ```text
//! kubectl get pods -o json | from json | where 'status.phase == "Running"'
//! cat /etc/passwd | parse '{user}:{x}:{uid}:{gid}:{name}:{home}:{shell}' | where 'uid > 1000'
//! ls -1 | lines | where 'line:match("%.rs$")'
//! ```
//!
//! **This is the half that makes structure useful today.** A shell whose structured world contains
//! only its own tools is five commands wide on the day it ships, and its author is the only user.
//! These three work with `kubectl`, `docker`, `gh`, `systemctl`, `cargo`, `jq` and anything else
//! already installed, with no cooperation from any of them.
//!
//! They are also the implementation strategy for every native tool that follows: oslo parses what
//! an existing tool prints rather than re-implementing what it knows.

use crate::data::{Record, Val};

/// Every line of the input as a one-column row.
///
/// The column is `line`, so `lines | where 'line:match("...")'` reads as a sentence. Blank lines
/// are kept — a blank line in `diff` output means something, and dropping them would be a decision
/// this has no business making.
pub fn lines(input: &str) -> Vec<Record> {
    input
        .lines()
        .map(|line| Record::from_pairs([("line", Val::Str(line.to_string()))]))
        .collect()
}

/// Rows from a pattern with `{name}` holes, one row per line.
///
/// `parse '{user}:{x}:{uid}'` against `bo:1000:1000` gives `user=bo`, `x=1000`, `uid=1000`. The
/// text between holes must match exactly; a line that does not match is skipped, because a file
/// with a comment header should not produce a row of nonsense.
///
/// A hole named `x` is still captured — naming a field you do not want is how you say "skip this
/// column" without inventing a syntax for it.
pub fn parse(input: &str, pattern: &str) -> Result<Vec<Record>, String> {
    let (names, separators) = compile(pattern)?;
    if names.is_empty() {
        return Err("parse: the pattern needs at least one {name}".to_string());
    }
    let mut out = Vec::new();
    'line: for line in input.lines() {
        let mut rest = line;
        let mut record = Record::new();
        for (i, name) in names.iter().enumerate() {
            // The literal text before this hole has to be there.
            let before = &separators[i];
            if !before.is_empty() {
                let Some(at) = rest.find(before.as_str()) else {
                    continue 'line;
                };
                rest = &rest[at + before.len()..];
            }
            // The hole runs up to the next literal, or to the end of the line for the last one.
            let after = separators.get(i + 1).cloned().unwrap_or_default();
            let value = if after.is_empty() {
                let value = rest;
                rest = "";
                value
            } else {
                let Some(at) = rest.find(after.as_str()) else {
                    continue 'line;
                };
                let value = &rest[..at];
                rest = &rest[at..];
                value
            };
            record.set(name, scalar(value));
        }
        out.push(record);
    }
    Ok(out)
}

/// The hole names, and the literal text before each of them plus whatever trails the last.
fn compile(pattern: &str) -> Result<(Vec<String>, Vec<String>), String> {
    let mut names = Vec::new();
    let mut separators = Vec::new();
    let mut literal = String::new();
    let mut chars = pattern.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '{' => {
                let mut name = String::new();
                for c in chars.by_ref() {
                    if c == '}' {
                        break;
                    }
                    name.push(c);
                }
                if name.is_empty() {
                    return Err("parse: a hole needs a name, as in {user}".to_string());
                }
                separators.push(std::mem::take(&mut literal));
                names.push(name);
            }
            other => literal.push(other),
        }
    }
    separators.push(literal);
    Ok((names, separators))
}

/// A captured field, as the most specific kind it plainly is.
///
/// A column of numbers should compare as numbers — `where 'uid > 1000'` is the point of parsing at
/// all. Anything that is not plainly a number stays text, because guessing is worse than not.
///
/// **Trimmed whichever kind it turns out to be.** The two numeric branches decided on the trimmed
/// text and the fallback then stored the untrimmed string, so a column-aligned line — the shape
/// `parse` exists for — gave `a` the number 42 and `b` the string `" x "`. `where 'b == "x"'` then
/// matched nothing, on output that plainly says `x`.
fn scalar(text: &str) -> Val {
    let trimmed = text.trim();
    if let Ok(i) = trimmed.parse::<i64>() {
        return Val::Int(i);
    }
    if let Ok(f) = trimmed.parse::<f64>()
        && trimmed.contains('.')
    {
        return Val::Float(f);
    }
    Val::Str(trimmed.to_string())
}

/// Rows from JSON: an array of objects becomes rows, anything else becomes one row.
pub fn from_json(input: &str) -> Result<Vec<Record>, String> {
    let parsed: serde_json::Value =
        serde_json::from_str(input.trim()).map_err(|e| format!("from json: {e}"))?;
    match parsed {
        serde_json::Value::Array(items) => Ok(items.iter().map(json_row).collect()),
        // A single object is one row. A bare scalar becomes a one-column row called `value`, so
        // that everything downstream can assume it is looking at rows.
        other => Ok(vec![json_row(&other)]),
    }
}

fn json_row(value: &serde_json::Value) -> Record {
    match value {
        serde_json::Value::Object(fields) => {
            let mut record = Record::new();
            for (name, value) in fields {
                record.set(name, json_value(value));
            }
            record
        }
        other => Record::from_pairs([("value", json_value(other))]),
    }
}

fn json_value(value: &serde_json::Value) -> Val {
    match value {
        serde_json::Value::Null => Val::Null,
        serde_json::Value::Bool(b) => Val::Bool(*b),
        serde_json::Value::Number(n) => match n.as_i64() {
            Some(i) => Val::Int(i),
            None => Val::Float(n.as_f64().unwrap_or(0.0)),
        },
        serde_json::Value::String(s) => Val::Str(s.clone()),
        serde_json::Value::Array(items) => Val::List(items.iter().map(json_value).collect()),
        serde_json::Value::Object(_) => Val::Record(json_row(value)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One column called `line`, and blank lines are somebody else's decision.
    #[test]
    fn lines_are_rows_of_one_column() {
        let rows = lines("a\n\nb\n");
        assert_eq!(rows.len(), 3, "the blank line is kept");
        assert_eq!(rows[0].get("line"), Some(&Val::Str("a".into())));
        assert_eq!(rows[1].get("line"), Some(&Val::Str("".into())));
    }

    /// The shape that makes `/etc/passwd` usable, including numbers arriving as numbers.
    #[test]
    fn a_pattern_captures_named_fields() {
        let rows = parse(
            "bo:x:1000:1000::/home/bo:/bin/sh",
            "{user}:{x}:{uid}:{rest}",
        )
        .expect("a valid pattern");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].get("user"), Some(&Val::Str("bo".into())));
        assert_eq!(
            rows[0].get("uid"),
            Some(&Val::Int(1000)),
            "numbers are numbers"
        );
        assert_eq!(rows[0].columns(), ["user", "x", "uid", "rest"]);
    }

    /// A line that does not fit the pattern is skipped rather than turned into nonsense.
    #[test]
    fn a_line_that_does_not_match_is_skipped() {
        let rows = parse("# a comment\nbo:1000\n", "{user}:{uid}").expect("valid");
        assert_eq!(rows.len(), 1, "only the line that fits");
        assert_eq!(rows[0].get("user"), Some(&Val::Str("bo".into())));
    }

    /// An empty pattern is a mistake worth reporting rather than an empty result.
    #[test]
    fn a_pattern_with_no_holes_is_refused() {
        assert!(parse("anything", "no holes here").is_err());
        assert!(parse("anything", "{}").is_err());
    }

    /// An array of objects is rows; anything else is one row.
    #[test]
    fn json_arrays_become_rows() {
        let rows = from_json(r#"[{"name":"a","n":1},{"name":"b","n":2}]"#).expect("valid json");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].get("name"), Some(&Val::Str("a".into())));
        assert_eq!(rows[1].get("n"), Some(&Val::Int(2)));

        let one = from_json(r#"{"only":true}"#).expect("valid json");
        assert_eq!(one.len(), 1);
        assert_eq!(one[0].get("only"), Some(&Val::Bool(true)));
    }

    /// Broken JSON is reported, not swallowed: a filter over nothing looks like a filter that
    /// matched nothing, which is the wrong thing to believe.
    #[test]
    fn broken_json_is_an_error() {
        assert!(from_json("{not json").is_err());
    }
}

#[cfg(test)]
mod scalar_tests {
    use super::{Val, scalar};

    /// **A captured field is trimmed whichever kind it turns out to be.**
    ///
    /// The two numeric branches decided on trimmed text and the fallback stored the untrimmed
    /// string, so a column-aligned line — the shape `parse` exists for — gave one column the number
    /// 42 and the next the string `" x "`. `where 'b == "x"'` then matched nothing, on output that
    /// plainly says `x`.
    #[test]
    fn a_field_is_trimmed_whether_it_is_a_number_or_not() {
        assert_eq!(scalar(" 42 "), Val::Int(42));
        assert_eq!(scalar(" 4.5 "), Val::Float(4.5));
        assert_eq!(scalar(" x "), Val::Str("x".to_string()));
        assert_eq!(scalar("\tname\t"), Val::Str("name".to_string()));
    }

    /// What is inside the field is untouched: only the padding around it goes.
    #[test]
    fn only_the_padding_goes() {
        assert_eq!(scalar("  two words  "), Val::Str("two words".to_string()));
        assert_eq!(scalar(""), Val::Str(String::new()));
        assert_eq!(scalar("   "), Val::Str(String::new()));
        // Not a number, because guessing is worse than not.
        assert_eq!(scalar(" 1x "), Val::Str("1x".to_string()));
        // A float needs the point, or `1e3` would stop being the text it is.
        assert_eq!(scalar(" 1e3 "), Val::Str("1e3".to_string()));
    }
}

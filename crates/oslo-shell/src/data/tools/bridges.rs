//! The four stages that turn **bytes into rows**: `lines`, `parse`, `from` and `detect-columns`.
//!
//! Split out of `super::run_tool`'s match for length, along the seam the planner already draws:
//! these are the `Bytes -> Rows` declarations, and every other arm of that match is `Rows -> Rows`
//! or `Rows -> Bytes`. What they share is that their input is a `&[u8]` somebody else wrote and
//! their failures are about *that document* rather than about the stream.
//!
//! The verbs themselves are in `bridge`, `formats` and `detect`; this is only the dispatch — which
//! operand selects what, and what each failure says.

use super::*;

/// One bridge stage, or `None` when `name` is not one of the four.
pub(super) fn run(name: &str, words: &[String], bytes: Option<&str>) -> Option<Outcome> {
    match name {
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
                        crate::env::complain(
                            words,
                            other,
                            &format!(
                                "detect-columns: {other}: not an option; it knows --no-headers and --skip"
                            ),
                            "not one of them",
                            Some(
                                "--no-headers reads row one as data; --skip N drops N lines first",
                            ),
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
                    crate::env::complain(
                        words,
                        other,
                        &format!("from: {other}: unknown format; oslo knows json, csv and tsv"),
                        "not a format",
                        Some(
                            "`from json`, `from csv`, `from tsv` — the format is an operand, not part of the name",
                        ),
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
        _ => None,
    }
}

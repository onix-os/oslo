//! The file itself: one `name=value` a line, and the escaping that keeps it that way.
//!
//! **A text format on purpose.** The store is something a person may open, read and fix by hand
//! after a bad edit, and every choice here follows from that: values are written as themselves,
//! only the three characters that would otherwise end a line are escaped, and a line that makes no
//! sense costs only itself rather than the file.

use super::Universal;
use std::collections::BTreeMap;

/// Encode a value so it cannot span lines.
///
/// Only three characters need it, and everything else is written as itself — which keeps the file
/// readable, and readable is the point of choosing a text format at all.
pub(super) fn escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            other => out.push(other),
        }
    }
    out
}

/// The inverse. An unknown escape keeps both characters rather than being dropped: losing data
/// silently is worse than a value that looks slightly wrong and can be corrected.
pub(super) fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('\\') => out.push('\\'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// Parse the whole file. A line that makes no sense is skipped rather than failing the read —
/// one corrupt line must not cost you every other variable.
pub(super) fn parse(text: &str) -> BTreeMap<String, Universal> {
    let mut out = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((flags, rest)) = line.split_once(' ') else {
            continue;
        };
        let Some((name, value)) = rest.split_once('=') else {
            continue;
        };
        if name.is_empty() {
            continue;
        }
        out.insert(
            name.to_string(),
            Universal {
                value: unescape(value),
                exported: flags.contains('x'),
            },
        );
    }
    out
}

pub(super) fn render(vars: &BTreeMap<String, Universal>) -> String {
    let mut out = String::new();
    // A header, because somebody will find this file and wonder. It is skipped on read like any
    // other comment.
    out.push_str("# oslo universal variables — written by `universal`, do not source\n");
    for (name, var) in vars {
        let flag = if var.exported { 'x' } else { '-' };
        out.push_str(&format!("{flag} {name}={}\n", escape(&var.value)));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A value containing the one character that would otherwise end its line survives.
    #[test]
    fn a_newline_in_a_value_does_not_end_the_line() {
        let mut vars = BTreeMap::new();
        vars.insert(
            "MULTI".to_string(),
            Universal {
                value: "one\ntwo\\three".to_string(),
                exported: false,
            },
        );
        let text = render(&vars);
        assert_eq!(
            text.lines().count(),
            2,
            "header plus one variable: {text:?}"
        );
        assert_eq!(parse(&text), vars, "must survive the round trip");
    }

    /// The export flag is part of the record, not something inferred on read.
    #[test]
    fn the_export_flag_round_trips() {
        let mut vars = BTreeMap::new();
        vars.insert(
            "SEEN".to_string(),
            Universal {
                value: "1".to_string(),
                exported: true,
            },
        );
        vars.insert(
            "HIDDEN".to_string(),
            Universal {
                value: "2".to_string(),
                exported: false,
            },
        );
        let back = parse(&render(&vars));
        assert!(back["SEEN"].exported);
        assert!(!back["HIDDEN"].exported);
    }

    /// One unreadable line costs that line and nothing else. A file this is rewritten into by
    /// every shell you have open must not become all-or-nothing.
    #[test]
    fn a_corrupt_line_does_not_cost_the_others() {
        let text = "# comment\n\
                    x GOOD=yes\n\
                    this line has no equals\n\
                    \n\
                    - ALSO=fine\n";
        let vars = parse(text);
        assert_eq!(vars.len(), 2, "{vars:?}");
        assert_eq!(vars["GOOD"].value, "yes");
        assert_eq!(vars["ALSO"].value, "fine");
    }

    /// An empty value is a value, and is different from the variable being absent.
    #[test]
    fn an_empty_value_is_kept() {
        let vars = parse("- EMPTY=\n");
        assert_eq!(vars["EMPTY"].value, "");
    }

    /// A value with an `=` in it keeps all of it — the split is at the *first* one.
    #[test]
    fn only_the_first_equals_separates() {
        let vars = parse("- URL=k=v&a=b\n");
        assert_eq!(vars["URL"].value, "k=v&a=b");
    }

    /// An escape nobody wrote is left alone rather than swallowed.
    #[test]
    fn an_unknown_escape_keeps_both_characters() {
        assert_eq!(unescape(r"a\qb"), r"a\qb");
        assert_eq!(unescape(r"trailing\"), r"trailing\");
        assert_eq!(unescape(r"\\"), r"\");
    }
}

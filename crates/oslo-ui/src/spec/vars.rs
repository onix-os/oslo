//! `${…}` inside a spec's values: the flags and arguments of the line being typed.
//!
//! ```yaml
//! completion:
//!   flag:
//!     suffix: ["$list(,)", ".go", ".md"]
//!   positional:
//!     - ["$files([${C_FLAG_SUFFIX//,/, }])"]   # what --suffix said, as a list
//!     - ["${C_FLAG_SUFFIX:-default}", "${C_ARG0}"]
//! ```
//!
//! carapace reaches for a general shell-substitution library here. This is the subset those specs
//! actually use, which is the subset that can be read at a glance:
//!
//! | form | means |
//! |---|---|
//! | `${NAME}` | the value, or nothing |
//! | `${NAME:-alt}` | `alt` when unset **or empty** |
//! | `${NAME-alt}` | `alt` when unset |
//! | `${NAME:+alt}` | `alt` when set and not empty |
//! | `${NAME/pat/rep}` | the first `pat` replaced |
//! | `${NAME//pat/rep}` | every `pat` replaced |
//!
//! **A bare `$name` is a macro, never a variable.** That is the one place this deliberately parts
//! company with a shell: `$files` has to stay `$files`, and a substituter that treated it as an
//! unset variable would quietly turn every macro in every spec into an empty string.

use super::Query;

/// Replace every `${…}` in `text` with what the line says it is.
pub fn expand(text: &str, query: &Query) -> String {
    if !text.contains("${") {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("${") {
        out.push_str(&rest[..at]);
        let after = &rest[at + 2..];
        let Some(end) = closing(after) else {
            // An opening with no `}` is text, not an expansion — a spec is edited by hand and
            // swallowing the rest of the line on a typo is the wrong way to say so.
            out.push_str(&rest[at..]);
            return out;
        };
        out.push_str(&one(&after[..end], query));
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

/// The offset of the `}` that closes an expansion opened at the start of `text`.
///
/// Counted rather than searched, so a replacement containing braces does not end it early.
fn closing(text: &str) -> Option<usize> {
    let mut depth = 0usize;
    for (at, ch) in text.char_indices() {
        match ch {
            '{' => depth += 1,
            '}' if depth == 0 => return Some(at),
            '}' => depth -= 1,
            _ => {}
        }
    }
    None
}

/// One expansion, without its braces.
fn one(body: &str, query: &Query) -> String {
    let cut = body
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(body.len());
    let (name, op) = body.split_at(cut);
    let value = query.variable(name);

    match op.as_bytes() {
        [] => value.unwrap_or_default(),
        [b':', b'-', ..] => or_else(value.filter(|v| !v.is_empty()), &op[2..]),
        [b'-', ..] => or_else(value, &op[1..]),
        [b':', b'+', ..] => match value.filter(|v| !v.is_empty()) {
            Some(_) => op[2..].to_string(),
            None => String::new(),
        },
        [b'/', b'/', ..] => replace(&value.unwrap_or_default(), &op[2..], usize::MAX),
        [b'/', ..] => replace(&value.unwrap_or_default(), &op[1..], 1),
        // Anything else is a form this does not know. The value alone is a better answer than the
        // raw text, which would reach the completer as a literal `${…}` offer.
        _ => value.unwrap_or_default(),
    }
}

fn or_else(value: Option<String>, alternative: &str) -> String {
    value.unwrap_or_else(|| alternative.to_string())
}

/// `pat/rep` applied to `value`, at most `limit` times.
fn replace(value: &str, spec: &str, limit: usize) -> String {
    let (pattern, replacement) = match spec.split_once('/') {
        Some(halves) => halves,
        // `${NAME//,}` deletes rather than replaces, which is what a shell does with it too.
        None => (spec, ""),
    };
    if pattern.is_empty() {
        return value.to_string();
    }
    value.replacen(pattern, replacement, limit)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn query() -> Query {
        let mut flags = std::collections::HashMap::new();
        flags.insert("SUFFIX".to_string(), ".go,.md".to_string());
        Query {
            args: vec!["first".into(), "second".into()],
            value: "part".into(),
            flags,
            ..Query::default()
        }
    }

    #[test]
    fn the_line_supplies_the_values() {
        let q = query();
        assert_eq!(expand("${C_ARG0}", &q), "first");
        assert_eq!(expand("${C_ARG1}-${C_VALUE}", &q), "second-part");
        assert_eq!(expand("${C_FLAG_SUFFIX}", &q), ".go,.md");
        assert_eq!(expand("${C_ARG9}", &q), "");
    }

    #[test]
    fn a_default_stands_in_for_what_was_not_typed() {
        let q = query();
        assert_eq!(expand("${C_ARG9:-none}", &q), "none");
        assert_eq!(expand("${C_ARG0:-none}", &q), "first");
        assert_eq!(expand("${C_ARG0:+yes}", &q), "yes");
        assert_eq!(expand("${C_ARG9:+yes}", &q), "");
    }

    /// The documented way to turn a `$list(,)` flag back into a macro's `[a, b]` argument.
    #[test]
    fn a_comma_list_becomes_a_bracket_list() {
        let q = query();
        assert_eq!(
            expand("$files([${C_FLAG_SUFFIX//,/, }])", &q),
            "$files([.go, .md])"
        );
        assert_eq!(expand("${C_FLAG_SUFFIX/,/;}", &q), ".go;.md");
    }

    /// **A macro is not a variable.** Substituting `$files` away is how every spec in the world
    /// would stop working at once.
    #[test]
    fn a_bare_dollar_is_left_alone() {
        let q = query();
        assert_eq!(expand("$files", &q), "$files");
        assert_eq!(expand("$directories", &q), "$directories");
        assert_eq!(expand("${unclosed", &q), "${unclosed");
    }
}

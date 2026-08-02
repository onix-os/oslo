//! `.env`: `KEY=value` lines, and nothing that can run.
//!
//! The reason this file type exists next to `.envrc` is that it *cannot execute anything*. It is the
//! format every other tool already writes — docker-compose, Rails, Django, `dotenv` — and its whole
//! value is that reading one is safe in a way that sourcing a shell script is not.
//!
//! That does not exempt it from the allow gate. `PATH=/tmp/evil:$PATH` is code execution with extra
//! steps, and `LD_PRELOAD` more directly still, so a `.env` is allowed exactly the way an `.envrc`
//! is. What the restricted grammar buys is that *reading* it cannot have side effects, so a file
//! that is refused costs nothing to have looked at.

/// Parse `.env` contents into pairs, in file order.
///
/// Deliberately small, and matching what the ecosystem actually writes rather than any one library's
/// full grammar:
///
/// * blank lines and `#` comments are skipped;
/// * a leading `export ` is allowed, because half the `.env` files in the world have it;
/// * single quotes are literal, double quotes expand the handful of escapes people actually use;
/// * an unquoted value has trailing whitespace and trailing comments stripped.
///
/// **No variable expansion.** `FOO=$BAR` yields the literal `$BAR`. Expansion is what `.envrc` is
/// for, and a half-expansion that handles `$BAR` but not `${BAR:-x}` or `$(cmd)` would be a worse
/// answer than none: it would look like it worked.
pub fn parse(source: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in source.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line).trim_start();
        let Some((name, value)) = line.split_once('=') else {
            continue;
        };
        let name = name.trim();
        if name.is_empty() || !is_name(name) {
            continue;
        }
        out.push((name.to_string(), value_of(value.trim())));
    }
    out
}

/// Whether this is a variable name a shell would accept.
///
/// Anything else is a line this format does not describe — and silently accepting it would put a
/// key in the environment that no expansion could ever read back.
fn is_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn value_of(raw: &str) -> String {
    if let Some(inner) = quoted(raw, '\'') {
        // Single quotes are literal in every shell there is, and in this format too.
        return inner.to_string();
    }
    if let Some(inner) = quoted(raw, '"') {
        return unescape(inner);
    }
    // Unquoted: a `#` after whitespace begins a comment. `A=a#b` keeps the hash, because that is a
    // value with a hash in it and not a comment — the same rule the shell lexer uses.
    match raw.split_once(" #") {
        Some((value, _)) => value.trim_end().to_string(),
        None => raw.to_string(),
    }
}

fn quoted(raw: &str, quote: char) -> Option<&str> {
    let rest = raw.strip_prefix(quote)?;
    // `len() >= 1` is not enough: a lone `"` starts and ends with the same character.
    rest.strip_suffix(quote)
}

/// The escapes people actually write in a `.env`, and no others.
fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('\\') => out.push('\\'),
            Some('"') => out.push('"'),
            // An escape this format does not define is left exactly as written, rather than having
            // its backslash eaten: a Windows path in a `.env` is common and must survive.
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(source: &str) -> Vec<(String, String)> {
        parse(source)
    }

    #[test]
    fn the_ordinary_shapes_all_read() {
        let pairs = parsed(
            "# a comment\n\
             \n\
             NAME=value\n\
             export EXPORTED=yes\n\
             SPACED = spaced out \n\
             QUOTED=\"in double\"\n\
             LITERAL='in single'\n",
        );
        assert_eq!(
            pairs,
            vec![
                ("NAME".into(), "value".into()),
                ("EXPORTED".into(), "yes".into()),
                ("SPACED".into(), "spaced out".into()),
                ("QUOTED".into(), "in double".into()),
                ("LITERAL".into(), "in single".into()),
            ]
        );
    }

    /// No expansion, and that is the promise. A half-expansion would be worse than none.
    #[test]
    fn nothing_is_expanded() {
        assert_eq!(
            parsed("A=$HOME\nB=${X:-y}\nC=$(id -u)\n"),
            vec![
                ("A".into(), "$HOME".into()),
                ("B".into(), "${X:-y}".into()),
                ("C".into(), "$(id -u)".into()),
            ]
        );
    }

    /// A hash inside a value is part of the value; a hash after whitespace is a comment.
    #[test]
    fn a_hash_only_starts_a_comment_after_whitespace() {
        assert_eq!(parsed("A=red#5\n"), vec![("A".into(), "red#5".into())]);
        assert_eq!(parsed("A=red # note\n"), vec![("A".into(), "red".into())]);
    }

    #[test]
    fn single_quotes_are_literal_and_double_quotes_escape() {
        assert_eq!(parsed(r#"A='a\nb'"#), vec![("A".into(), r"a\nb".into())]);
        assert_eq!(parsed(r#"A="a\nb""#), vec![("A".into(), "a\nb".into())]);
    }

    /// A backslash this format does not define keeps its backslash, or Windows paths break.
    #[test]
    fn an_unknown_escape_survives_intact() {
        assert_eq!(
            parsed(r#"A="C:\Users\me""#),
            vec![("A".into(), r"C:\Users\me".into())]
        );
    }

    #[test]
    fn lines_that_are_not_assignments_are_skipped() {
        assert_eq!(
            parsed("just a line\n1BAD=x\nBAD-NAME=x\n=novalue\n"),
            vec![]
        );
    }
}

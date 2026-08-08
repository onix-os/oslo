//! Turning a value back into shell source.
//!
//! Every listing builtin (`export -p`, `readonly -p`, `alias`, `set`) promises that its output
//! can be fed back to a shell. That promise is only worth something if the *value* is quoted:
//! `export PS1=$ ` and `alias q=echo 'a'` are not assignments, they are syntax errors or worse.
//! Printing values raw — or with Rust's `{:?}`, which escapes for Rust and not for sh — is what
//! these helpers replace.

/// Characters that need no quoting anywhere in a word.
///
/// Deliberately conservative: `~` and `#` are safe in the middle of a word but not at the start,
/// and the cost of quoting them anyway is one pair of quotes.
fn is_bare_safe(c: char) -> bool {
    c.is_ascii_alphanumeric() || "_@%+=:,./-".contains(c)
}

/// Wrap `value` in single quotes, whatever it contains.
///
/// A single quote cannot be escaped *inside* single quotes, so the only way to carry one is to
/// leave the quoted run, emit an escaped quote, and start a new run: `it's` becomes
/// `'it'\''s'`. A newline needs nothing special — it is literal inside single quotes, and the
/// listing simply spans two lines, exactly as bash's does.
pub fn single_quoted(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// Quote `value` only as much as it needs, the way bash's `set` listing does.
///
/// Three forms, in order of preference: bare when every character is safe, `$'...'` when a
/// control character would otherwise be written literally into the middle of a listing, and
/// single quotes for everything else.
pub fn quote_minimal(value: &str) -> String {
    if !value.is_empty() && value.chars().all(is_bare_safe) {
        value.to_string()
    } else if value.chars().any(char::is_control) {
        ansi_c_quoted(value)
    } else {
        single_quoted(value)
    }
}

/// Render `value` as `$'...'`, the only quoting form that can hold a control character on one
/// line.
///
/// Used rather than a literal newline so that a listing stays one variable per line: a script
/// that reads `set` output line by line — and there are many — otherwise sees a value split
/// across records.
fn ansi_c_quoted(value: &str) -> String {
    let mut out = String::from("$'");
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\'"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if c.is_control() => out.push_str(&format!("\\x{:02x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('\'');
    out
}

#[cfg(test)]
mod tests {
    use super::{quote_minimal, single_quoted};

    #[test]
    fn an_embedded_quote_leaves_and_re_enters_the_quoted_run() {
        assert_eq!(single_quoted("it's"), r"'it'\''s'");
        assert_eq!(single_quoted("'"), r"''\'''");
        assert_eq!(single_quoted(""), "''");
    }

    #[test]
    fn safe_words_are_left_bare_and_the_rest_are_quoted() {
        assert_eq!(quote_minimal("plain"), "plain");
        assert_eq!(quote_minimal("/usr/bin:/bin"), "/usr/bin:/bin");
        assert_eq!(quote_minimal("a b"), "'a b'");
        assert_eq!(quote_minimal(""), "''");
        assert_eq!(quote_minimal("$HOME"), "'$HOME'");
    }

    /// A newline must not end up as a raw newline in a listing that is read line by line.
    #[test]
    fn control_characters_use_the_ansi_c_form() {
        assert_eq!(quote_minimal("a\nb"), r"$'a\nb'");
        assert_eq!(quote_minimal("a\tb"), r"$'a\tb'");
        assert_eq!(quote_minimal("\x01"), r"$'\x01'");
        assert!(!quote_minimal("a\nb").contains('\n'));
    }
}

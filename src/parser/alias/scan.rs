//! The character-level half of alias substitution: where a word ends, what a name may be, and
//! how to copy a construct through untouched.
//!
//! Split from the scanner above it because these answer questions about *text* — quoting, word
//! boundaries, balanced brackets — while the scanner answers a question about *grammar*: whether
//! the word it is looking at begins a command.

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Quote {
    None,
    Single,
    Double,
}

/// Where the word starting at `start` ends: at a blank or an unquoted shell metacharacter.
pub(super) fn word_end(chars: &[char], start: usize) -> usize {
    let mut i = start;
    let mut quote = Quote::None;
    while i < chars.len() {
        let c = chars[i];
        match quote {
            Quote::Single => {
                if c == '\'' {
                    quote = Quote::None;
                }
            }
            Quote::Double => {
                if c == '"' {
                    quote = Quote::None;
                }
            }
            Quote::None => match c {
                '\'' => quote = Quote::Single,
                '"' => quote = Quote::Double,
                '\\' => i += 1,
                ' ' | '\t' | ';' | '&' | '|' | '(' | ')' | '<' | '>' | '#' | '`' => return i,
                _ => {}
            },
        }
        i += 1;
    }
    chars.len()
}

/// Whether the next non-blank characters are `()`, which makes the word a function's name.
pub(super) fn is_function_definition(chars: &[char], from: usize) -> bool {
    let mut i = from;
    while i < chars.len() && (chars[i] == ' ' || chars[i] == '\t') {
        i += 1;
    }
    chars.get(i) == Some(&'(')
}

/// Whether a word can be an alias name at all.
///
/// POSIX allows more than this, but a name containing a quote, an expansion or a metacharacter
/// cannot be looked up without expanding it first — and expanding it is not this pass's job.
pub(super) fn is_plain_name(word: &str) -> bool {
    !word.is_empty()
        && !word.contains(['\'', '"', '$', '`', '\\', '=', '/'])
        && word != "!"
        && !super::INTRODUCERS.contains(&word)
}

/// `name=value`, the shape of a command prefix.
pub(super) fn is_assignment(word: &str) -> bool {
    match word.split_once('=') {
        Some((name, _)) => {
            !name.is_empty()
                && name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '[' || c == ']')
        }
        None => false,
    }
}

/// Split a line into words, keeping quotes attached, for reading `alias` operands.
pub(super) fn split_words(line: &str) -> Vec<String> {
    let chars: Vec<char> = line.chars().collect();
    let mut words = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            ' ' | '\t' => i += 1,
            '#' => break,
            ';' | '&' | '|' => {
                let start = i;
                while i < chars.len() && chars[i] == chars[start] {
                    i += 1;
                }
                words.push(chars[start..i].iter().collect());
            }
            _ => {
                let end = word_end(&chars, i);
                if end == i {
                    i += 1;
                    continue;
                }
                words.push(chars[i..end].iter().collect());
                i = end;
            }
        }
    }
    words
}

/// Strip one layer of surrounding quotes from an alias body, as the `alias` builtin would.
pub(super) fn unquote(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars().peekable();
    let mut quote = Quote::None;
    while let Some(c) = chars.next() {
        match quote {
            Quote::Single => {
                if c == '\'' {
                    quote = Quote::None;
                } else {
                    out.push(c);
                }
            }
            Quote::Double => match c {
                '"' => quote = Quote::None,
                '\\' => {
                    if let Some(next) = chars.next() {
                        out.push(next);
                    }
                }
                _ => out.push(c),
            },
            Quote::None => match c {
                '\'' => quote = Quote::Single,
                '"' => quote = Quote::Double,
                '\\' => {
                    if let Some(next) = chars.next() {
                        out.push(next);
                    }
                }
                _ => out.push(c),
            },
        }
    }
    out
}

/// Copy a balanced `open`/`close` run through untouched, quotes and all.
///
/// `from` points at the first `open`. Returns the index just past the matching close, or the
/// end of the text when it is never closed — an unterminated construct is a syntax error the
/// parser will report, and this pass must not hang on it.
pub(super) fn copy_balanced(
    out: &mut String,
    chars: &[char],
    from: usize,
    open: char,
    close: char,
) -> usize {
    let mut depth = 0usize;
    let mut i = from;
    let mut quote = Quote::None;
    while i < chars.len() {
        let c = chars[i];
        out.push(c);
        i += 1;
        match quote {
            Quote::Single => {
                if c == '\'' {
                    quote = Quote::None;
                }
                continue;
            }
            Quote::Double => {
                if c == '"' {
                    quote = Quote::None;
                } else if c == '\\' && i < chars.len() {
                    out.push(chars[i]);
                    i += 1;
                }
                continue;
            }
            Quote::None => {}
        }
        match c {
            '\'' => quote = Quote::Single,
            '"' => quote = Quote::Double,
            '\\' if i < chars.len() => {
                out.push(chars[i]);
                i += 1;
            }
            c if c == open => depth += 1,
            c if c == close => {
                depth -= 1;
                if depth == 0 {
                    return i;
                }
            }
            _ => {}
        }
    }
    i
}

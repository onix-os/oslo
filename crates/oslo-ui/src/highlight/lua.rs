//! Painting a **Lua** line, which is not a shell line.
//!
//! Until this existed the Lua prompt was painted with the shell's rules, and the result was worse
//! than no colour at all: `print` is not on `$PATH` and `nil` is not a command, so both were drawn
//! in the *unknown command* style — red and underlined, the same as a typo. Every valid Lua line
//! looked like an error, on the prompt whose whole job is Lua.
//!
//! # What it colours, and what it deliberately does not
//!
//! Keywords, strings, comments and numbers, which are decided by the text alone. Names are left
//! plain unless the session actually has one, and *that* is looked up rather than guessed — a name
//! that exists is drawn as a function or a table, and one that does not is left alone rather than
//! marked wrong. A Lua prompt is where you define things: `x` is not an error just because `x` has
//! not been assigned yet, which is exactly the mistake the shell rules were making.
//!
//! Nothing here parses. A highlighter runs on every keystroke against a line that is usually
//! half-typed and often does not parse at all, so it reads tokens and stops — an unterminated
//! string colours to the end of the line, which is what it looks like to a reader too.

use crate::theme::{Style, Theme};

/// Lua's reserved words.
const KEYWORDS: &[&str] = &[
    "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "goto", "if", "in",
    "local", "nil", "not", "or", "repeat", "return", "then", "true", "until", "while",
];

/// Paint one line of Lua.
///
/// `known` answers whether a bare name exists in the session and whether it is callable, so a
/// global can be drawn as what it is. It is asked once per name, and only for names that are not
/// keywords.
pub fn paint(
    line: &str,
    theme: &Theme,
    depth: crate::theme::Depth,
    known: &dyn Fn(&str) -> Option<bool>,
) -> String {
    let syntax = &theme.syntax;
    let mut out = String::new();
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let rest = &line[i..];
        // A comment runs to the end of the line; a long one (`--[[`) does too, as far as this is
        // concerned, because the rest of the block has not been typed yet.
        if rest.starts_with("--") {
            out.push_str(&syntax.comment.paint(rest, depth));
            break;
        }
        let c = bytes[i] as char;
        if c == '"' || c == '\'' {
            let end = string_end(line, i, c);
            let style = if c == '\'' {
                &syntax.single_quote
            } else {
                &syntax.double_quote
            };
            out.push_str(&style.paint(&line[i..end], depth));
            i = end;
            continue;
        }
        if rest.starts_with("[[") {
            out.push_str(&syntax.double_quote.paint(rest, depth));
            break;
        }
        if c.is_ascii_digit() {
            let end = number_end(line, i);
            out.push_str(&syntax.number.paint(&line[i..end], depth));
            i = end;
            continue;
        }
        if c.is_ascii_alphabetic() || c == '_' {
            let end = name_end(line, i);
            let word = &line[i..end];
            out.push_str(&style_for(word, line, end, syntax, known).paint(word, depth));
            i = end;
            continue;
        }
        if is_operator(c) {
            out.push_str(&syntax.operator.paint(&c.to_string(), depth));
            i += c.len_utf8();
            continue;
        }
        out.push(c);
        i += c.len_utf8();
    }
    out
}

/// Which style a bare word takes.
///
/// A keyword is a keyword. Otherwise the session is asked: a callable name is a function, a name
/// that exists at all is a table or a value, and a name nothing knows is left **plain** — a Lua
/// prompt is where names are brought into being, so an unknown one is a normal thing to be typing
/// and not a mistake to be marked.
///
/// A name directly followed by `(` is drawn as a call even when nothing knows it yet, because the
/// text says it is one.
fn style_for<'a>(
    word: &str,
    line: &str,
    end: usize,
    syntax: &'a crate::theme::Syntax,
    known: &dyn Fn(&str) -> Option<bool>,
) -> &'a Style {
    if KEYWORDS.contains(&word) {
        return &syntax.keyword;
    }
    let called = line[end..].trim_start().starts_with('(');
    match known(word) {
        Some(true) => &syntax.function,
        Some(false) => &syntax.builtin,
        None if called => &syntax.function,
        None => &syntax.param,
    }
}

/// Where the string opened at `at` ends, one past its closing quote — or the end of the line when
/// it never closes, which is the usual case while it is being typed.
fn string_end(line: &str, at: usize, quote: char) -> usize {
    let bytes = line.as_bytes();
    let mut i = at + 1;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            i += 2;
            continue;
        }
        if bytes[i] as char == quote {
            return (i + 1).min(bytes.len());
        }
        i += 1;
    }
    bytes.len()
}

/// Where the number starting at `at` ends. Hex, exponents and `_` separators all read as one.
fn number_end(line: &str, at: usize) -> usize {
    let bytes = line.as_bytes();
    let mut i = at;
    while i < bytes.len() {
        let c = bytes[i] as char;
        if c.is_ascii_alphanumeric() || c == '.' || c == '_' {
            i += 1;
            continue;
        }
        // The sign of an exponent, which is part of the number rather than an operator.
        if (c == '+' || c == '-') && matches!(bytes[i - 1] as char, 'e' | 'E' | 'p' | 'P') {
            i += 1;
            continue;
        }
        break;
    }
    i
}

fn name_end(line: &str, at: usize) -> usize {
    let bytes = line.as_bytes();
    let mut i = at;
    while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
        i += 1;
    }
    i
}

fn is_operator(c: char) -> bool {
    matches!(
        c,
        '+' | '-' | '*' | '/' | '%' | '^' | '#' | '&' | '~' | '|' | '<' | '>' | '=' | '.'
    )
}

#[cfg(test)]
#[path = "lua/tests.rs"]
mod tests;

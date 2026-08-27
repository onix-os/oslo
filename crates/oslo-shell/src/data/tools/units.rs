//! Unit literals in a filter: `where 'size > 1GB'`.
//!
//! ```text
//! du | where 'size > 1GB'          1GB  →  1000000000
//! ps | where 'cpu_time > 5min'     5min →  300000000000
//! ```
//!
//! # Why a rewrite rather than a Lua binding
//!
//! `1GB` is not Lua and cannot be made into Lua: a numeral followed by a name is a syntax error, so
//! there is no function, metatable or global that could give it meaning. Today it reports
//! `found "Name(\"GB\")", expected "RightParen"` — a parser's complaint about a filter that reads
//! perfectly well to a person. The only way to accept what people write is to replace the literal
//! with its number before the expression is compiled.
//!
//! # Which number
//!
//! The one the rows already carry, so a comparison means what it looks like. A `Val::Size` reaches
//! Lua as **bytes** and a `Val::Duration` as **nanoseconds**, so a literal is converted to whichever
//! of those it can be:
//!
//! ```text
//! 1GB   → `1GB in bytes` → 1000000000
//! 1GiB  → `1GiB in bytes` → 1073741824      the binary one, kept distinct
//! 5min  → `5min in ns`   → 300000000000
//! ```
//!
//! Bytes are tried first because a data size is what a filter usually compares, and nothing is both.
//! A literal neither conversion accepts is left exactly as it was, so an expression this does not
//! understand fails the way it always did rather than in some new way.
//!
//! # What it must not touch
//!
//! The scan is deliberately narrow, because every one of these is a filter somebody has written:
//!
//! | text | left alone because |
//! |---|---|
//! | `1e3` | it is already a number — Rust parses it, so it is not a unit literal |
//! | `0x1f` | a hex numeral, which Lua reads and this must not take apart |
//! | `x1GB` | a name; a digit preceded by an identifier character starts nothing |
//! | `'1GB'` | inside quotes — a filter comparing strings is comparing strings |
//! | `1QQ` | the calculator refuses it, so it stays and Lua reports it as it would have |

/// Replace every unit literal in `expression` with the number the rows carry.
pub fn expand(expression: &str) -> String {
    let bytes = expression.as_bytes();
    // **Bytes, not chars.** This walks the expression a byte at a time, and a byte pushed onto a
    // `String` as `byte as char` is re-encoded as Latin-1: every non-ASCII character in a filter
    // came out as the two or three characters its UTF-8 happened to be, so
    // `where 'name == "café"'` compared against something no row could ever hold and silently
    // matched nothing. Copying whole bytes in order preserves the encoding exactly; the result is
    // valid UTF-8 because the input was.
    let mut out: Vec<u8> = Vec::with_capacity(expression.len());
    let mut at = 0;
    let mut quote: Option<u8> = None;

    while at < bytes.len() {
        let byte = bytes[at];

        // Inside a string a unit literal is text, and a person comparing `name == '1GB'` means the
        // characters. Escapes are stepped over so `'\''` does not read as the end.
        if let Some(open) = quote {
            out.push(byte);
            at += 1;
            if byte == b'\\' && at < bytes.len() {
                out.push(bytes[at]);
                at += 1;
            } else if byte == open {
                quote = None;
            }
            continue;
        }
        if byte == b'\'' || byte == b'"' {
            quote = Some(byte);
            out.push(byte);
            at += 1;
            continue;
        }

        match literal_at(expression, at) {
            Some((text, end)) => match number_for(text) {
                Some(number) => {
                    out.extend_from_slice(number.as_bytes());
                    at = end;
                }
                // The calculator does not know it, so it is not ours: leave it and let Lua say
                // whatever it would have said.
                None => {
                    out.extend_from_slice(text.as_bytes());
                    at = end;
                }
            },
            None => {
                out.push(byte);
                at += 1;
            }
        }
    }
    // Valid by construction: whole bytes of a valid string, in order.
    String::from_utf8_lossy(&out).into_owned()
}

/// The unit literal starting at `at`, and where it ends.
///
/// The numeral is measured **first and in full** — digits, a decimal point, and an exponent with
/// its sign — and only then is a following letter a unit. That order is the whole of the safety:
/// scanning digits and grabbing whatever letters came next read `1e3` as one-and-`e`, asked the
/// calculator for Euler's number in bytes, and quietly rewrote a working filter to
/// `size > 0.339785228563`.
fn literal_at(expression: &str, at: usize) -> Option<(&str, usize)> {
    let bytes = expression.as_bytes();
    if !bytes[at].is_ascii_digit() {
        return None;
    }
    // A digit inside a name — `x1GB`, `col2` — begins nothing.
    if at > 0
        && (bytes[at - 1].is_ascii_alphanumeric() || bytes[at - 1] == b'_' || bytes[at - 1] == b'.')
    {
        return None;
    }
    // `0x…` is a numeral Lua reads; taking it apart would change what it means.
    if bytes[at] == b'0' && matches!(bytes.get(at + 1), Some(b'x' | b'X')) {
        return None;
    }

    let mut end = at;
    while end < bytes.len() && (bytes[end].is_ascii_digit() || bytes[end] == b'.') {
        end += 1;
    }
    // An exponent only counts when digits actually follow it, so the `e` of `1e` stays a letter.
    if let Some(b'e' | b'E') = bytes.get(end) {
        let mut after = end + 1;
        if matches!(bytes.get(after), Some(b'+' | b'-')) {
            after += 1;
        }
        if bytes.get(after).is_some_and(u8::is_ascii_digit) {
            while after < bytes.len() && bytes[after].is_ascii_digit() {
                after += 1;
            }
            end = after;
        }
    }
    let number = end;

    // One optional space, because `1 GB` is how people write it and `math` reads it that way too.
    let mut unit = end;
    if matches!(bytes.get(unit), Some(b' ')) {
        unit += 1;
    }
    let name = unit;
    while unit < bytes.len() && bytes[unit].is_ascii_alphabetic() {
        unit += 1;
    }
    if unit == name {
        return None;
    }
    // A letter running into a name — `1 GB_x`, `1GBx2` — is not a unit literal.
    if matches!(bytes.get(unit), Some(b'_')) || bytes.get(unit).is_some_and(u8::is_ascii_digit) {
        return None;
    }

    // **A Lua keyword is not a unit.** `n > 1 and m` is valid Lua and the only place a numeral is
    // legally followed by a word; `and`, `or` and `not` are the words it can be.
    const KEYWORDS: &[&str] = &[
        "and", "or", "not", "then", "do", "end", "else", "elseif", "if", "while", "for", "in",
        "function", "local", "return", "nil", "true", "false", "repeat", "until", "break",
    ];
    if KEYWORDS.contains(&&expression[name..unit]) {
        return None;
    }

    let text = &expression[at..unit];
    // **Already a number.** `1e3` and `1.5e-3` are Lua numerals, and the numeral scan above means
    // no letter was taken from them — this catches whatever it did not.
    if expression[at..number].parse::<f64>().is_ok() && number == unit {
        return None;
    }
    Some((text, unit))
}

/// The literal as the number a row carries, or `None` if the calculator does not know it.
#[cfg(feature = "math")]
fn number_for(text: &str) -> Option<String> {
    // Bytes first: a data size is what a filter usually compares, and nothing is both a size and a
    // duration, so the order decides nothing except which question is asked first.
    for base in ["bytes", "ns"] {
        if let Ok(answer) = oslo_math::calculate(&format!("{text} in {base}")) {
            return Some(oslo_math::format::number_text(answer.number));
        }
    }
    None
}

/// Without the calculator there is nothing to ask, so every literal stays as it was.
#[cfg(not(feature = "math"))]
fn number_for(_text: &str) -> Option<String> {
    None
}

#[cfg(test)]
#[path = "units/tests.rs"]
mod tests;

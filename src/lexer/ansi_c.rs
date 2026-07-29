//! ANSI-C quoting: decoding the body of `$'…'`.
//!
//! The decoded text is quoted data — it is handed back as a single-quoted word part, so no
//! expansion, field splitting or globbing may touch it afterwards. That is the whole reason
//! `IFS=$'\n'` has to be decoded here rather than left for the expander: the newline the user
//! asked for must survive as *one* literal character.

/// Decode `$'…'` escapes.
///
/// The body arrives with backslash sequences intact (the scanner only used them to know which
/// `'` closes the string). An unrecognised escape keeps both characters, which is what bash does.
///
/// Decoding stops at a NUL: a shell variable is a C string, so bash's `$'a\0b'` is one character
/// long. Producing the byte and losing it later would be worse — it would reach `CString` and
/// fail at exec time.
pub(super) fn decode(raw: &str) -> String {
    let chars: Vec<char> = raw.chars().collect();
    let mut out = String::new();
    let mut i = 0;

    while i < chars.len() {
        if chars[i] != '\\' || i + 1 >= chars.len() {
            out.push(chars[i]);
            i += 1;
            continue;
        }

        let esc = chars[i + 1];
        i += 2;
        let simple = match esc {
            'a' => Some('\x07'),
            'b' => Some('\x08'),
            'e' | 'E' => Some('\x1b'),
            'f' => Some('\x0c'),
            'n' => Some('\n'),
            'r' => Some('\r'),
            't' => Some('\t'),
            'v' => Some('\x0b'),
            '\\' => Some('\\'),
            '\'' => Some('\''),
            '"' => Some('"'),
            '?' => Some('?'),
            _ => None,
        };
        if let Some(c) = simple {
            out.push(c);
            continue;
        }

        let decoded = match esc {
            // `\0nnn` spends its `0` as an introducer, so three digits still follow it; `\nnn`
            // has already spent one of its three. That is why `\0101` is `A` but `\1011` is `A1`.
            '0' => radix_escape(&chars, &mut i, Some(0), 0, 8, 3),
            '1'..='7' => radix_escape(&chars, &mut i, esc.to_digit(8), 1, 8, 3),
            'x' => radix_escape(&chars, &mut i, None, 0, 16, 2),
            'u' => radix_escape(&chars, &mut i, None, 0, 16, 4),
            'U' => radix_escape(&chars, &mut i, None, 0, 16, 8),
            // `\cX` is the control character X masks down to; `\c\` is not special-cased because
            // the byte it produces (NUL) would truncate the string anyway.
            'c' => match chars.get(i) {
                Some(&c) => {
                    i += 1;
                    Some((c.to_ascii_uppercase() as u32) ^ 0x40)
                }
                None => None,
            },
            _ => None,
        };

        match decoded {
            // A shell string cannot carry a NUL, so everything from here on is unreachable data.
            Some(0) => return out,
            Some(v) => match char::from_u32(v) {
                Some(c) => out.push(c),
                // Lone surrogates and out-of-range code points: keep the source text rather than
                // inventing a replacement character the user never wrote.
                None => {
                    out.push('\\');
                    out.push(esc);
                }
            },
            None => {
                out.push('\\');
                out.push(esc);
            }
        }
    }

    out
}

/// Consume up to `max - taken` further digits in `radix`, folding them onto `seed`.
///
/// Returns `None` when there is nothing to read at all, so the caller can fall back to keeping
/// the escape verbatim the way bash does for a bare `\x`.
fn radix_escape(
    chars: &[char],
    i: &mut usize,
    seed: Option<u32>,
    taken: usize,
    radix: u32,
    max: usize,
) -> Option<u32> {
    let mut value = seed;
    let mut taken = taken;

    while taken < max {
        match chars.get(*i).and_then(|c| c.to_digit(radix)) {
            Some(d) => {
                value = Some(value.unwrap_or(0) * radix + d);
                *i += 1;
                taken += 1;
            }
            None => break,
        }
    }

    value
}

#[cfg(test)]
mod tests {
    use super::decode;

    #[test]
    fn control_escapes() {
        assert_eq!(decode("a\\tb"), "a\tb");
        assert_eq!(decode("l1\\nl2"), "l1\nl2");
        assert_eq!(decode("\\r"), "\r");
        assert_eq!(decode("\\\\"), "\\");
        assert_eq!(decode("\\'"), "'");
        assert_eq!(decode("\\\""), "\"");
        assert_eq!(decode("\\e"), "\x1b");
    }

    #[test]
    fn numeric_escapes() {
        assert_eq!(decode("\\101"), "A");
        assert_eq!(decode("\\0101"), "A");
        assert_eq!(decode("\\x41"), "A");
        assert_eq!(decode("\\x4"), "\u{4}");
        assert_eq!(decode("\\u00e9"), "é");
        assert_eq!(decode("\\U0001F600"), "\u{1F600}");
        assert_eq!(decode("\\cA"), "\x01");
    }

    /// An octal escape is at most three digits, so the fourth character is data again.
    #[test]
    fn an_octal_escape_stops_at_three_digits() {
        assert_eq!(decode("\\1011"), "A1");
    }

    /// bash keeps both characters of an escape it does not know; dropping the backslash would
    /// silently rewrite `$'\d'` into `d`.
    #[test]
    fn an_unknown_escape_is_kept_verbatim() {
        assert_eq!(decode("\\d"), "\\d");
        assert_eq!(decode("\\x"), "\\x");
        assert_eq!(decode("trailing\\"), "trailing\\");
    }

    /// A shell string is a C string: `$'a\0b'` is one character in bash too.
    #[test]
    fn a_nul_truncates_the_string() {
        assert_eq!(decode("a\\0b"), "a");
        assert_eq!(decode("a\\x00b"), "a");
    }
}

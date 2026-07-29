//! `echo`: the leading option run and the `-e` escape table.

use crate::env::scope::Environment;
use crate::error::Result;

/// What the leading `-n`/`-e`/`-E` run left behind.
struct EchoOptions {
    /// `-n` clears this.
    newline: bool,
    /// `-e` sets it, `-E` clears it again; the last one in the run wins.
    escapes: bool,
}

/// Whether an argument is part of the option run.
///
/// Only a `-` followed by one or more of `neE` counts. `echo -x`, `echo --` and `echo -` are all
/// data in bash, and the scan stops at the first of them — which is why the check is
/// all-or-nothing rather than per-character.
fn is_option(arg: &str) -> bool {
    match arg.strip_prefix('-') {
        Some(flags) => !flags.is_empty() && flags.chars().all(|c| matches!(c, 'n' | 'e' | 'E')),
        None => false,
    }
}

/// Append the byte an escape sequence denotes, returning the index just past it.
///
/// `chars[i]` is the character after the backslash. An unrecognised sequence is not an error:
/// bash emits the backslash and the character unchanged, so `\q` stays `\q`.
fn push_escape(out: &mut Vec<u8>, chars: &[char], i: usize) -> usize {
    let simple = match chars[i] {
        'a' => Some(0x07),
        'b' => Some(0x08),
        'e' => Some(0x1b),
        'f' => Some(0x0c),
        'n' => Some(b'\n'),
        'r' => Some(b'\r'),
        't' => Some(b'\t'),
        'v' => Some(0x0b),
        '\\' => Some(b'\\'),
        _ => None,
    };
    if let Some(byte) = simple {
        out.push(byte);
        return i + 1;
    }

    match chars[i] {
        // `\0nnn`: the leading zero is part of the syntax, then up to three octal digits.
        '0' => {
            let mut end = i + 1;
            let mut value: u32 = 0;
            while end < chars.len() && end < i + 4 && chars[end].is_digit(8) {
                value = value * 8 + chars[end].to_digit(8).unwrap();
                end += 1;
            }
            out.push(value as u8);
            end
        }
        // `\xHH`: one or two hex digits. With none at all the sequence is not an escape.
        'x' => {
            let mut end = i + 1;
            let mut value: u32 = 0;
            while end < chars.len() && end < i + 3 && chars[end].is_ascii_hexdigit() {
                value = value * 16 + chars[end].to_digit(16).unwrap();
                end += 1;
            }
            if end == i + 1 {
                out.extend_from_slice(b"\\x");
            } else {
                out.push(value as u8);
            }
            end
        }
        other => {
            out.push(b'\\');
            let mut buf = [0u8; 4];
            out.extend_from_slice(other.encode_utf8(&mut buf).as_bytes());
            i + 1
        }
    }
}

/// Interpret `-e` escapes. The flag reports `\c`, which truncates the output *and* suppresses
/// the trailing newline — so it cannot be expressed by the returned bytes alone.
///
/// The result is bytes rather than a `String` because `\xHH` above 0x7f denotes one raw byte,
/// which UTF-8 would silently widen to two.
fn expand_escapes(text: &str) -> (Vec<u8>, bool) {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '\\' {
            let mut buf = [0u8; 4];
            out.extend_from_slice(chars[i].encode_utf8(&mut buf).as_bytes());
            i += 1;
            continue;
        }
        // A backslash with nothing after it is data.
        if i + 1 == chars.len() {
            out.push(b'\\');
            break;
        }
        if chars[i + 1] == 'c' {
            return (out, true);
        }
        i = push_escape(&mut out, &chars, i + 1);
    }
    (out, false)
}

/// `echo [-neE] [arg…]` — arguments joined by a single space.
pub fn builtin_echo(_env: &mut Environment, args: &[String]) -> Result<i32> {
    let mut opts = EchoOptions {
        newline: true,
        escapes: false,
    };
    let mut idx = 1;
    while idx < args.len() && is_option(&args[idx]) {
        for flag in args[idx][1..].chars() {
            match flag {
                'n' => opts.newline = false,
                'e' => opts.escapes = true,
                'E' => opts.escapes = false,
                _ => unreachable!("is_option admits only neE"),
            }
        }
        idx += 1;
    }

    let joined = args[idx..].join(" ");
    let (mut output, truncated) = if opts.escapes {
        expand_escapes(&joined)
    } else {
        (joined.into_bytes(), false)
    };
    if opts.newline && !truncated {
        output.push(b'\n');
    }

    let _ = nix::unistd::write(unsafe { std::os::fd::BorrowedFd::borrow_raw(1) }, &output);
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::{expand_escapes, is_option};

    fn escaped(text: &str) -> String {
        String::from_utf8(expand_escapes(text).0).expect("ascii fixture")
    }

    #[test]
    fn only_runs_of_n_e_and_capital_e_are_options() {
        assert!(is_option("-n"));
        assert!(is_option("-neE"));
        assert!(!is_option("-x"));
        assert!(!is_option("--"));
        assert!(!is_option("-"));
        assert!(!is_option("n"));
    }

    #[test]
    fn the_control_escapes_match_the_table() {
        assert_eq!(escaped("a\\tb"), "a\tb");
        assert_eq!(
            escaped("\\a\\b\\e\\f\\n\\r\\t\\v"),
            "\x07\x08\x1b\x0c\n\r\t\x0b"
        );
        assert_eq!(escaped("a\\\\b"), "a\\b");
    }

    /// bash prints an unknown sequence verbatim rather than dropping the backslash.
    #[test]
    fn an_unknown_escape_survives_intact() {
        assert_eq!(escaped("\\q"), "\\q");
        assert_eq!(escaped("\\8"), "\\8");
        assert_eq!(escaped("\\xZ"), "\\xZ");
        // A trailing backslash has nothing to escape.
        assert_eq!(escaped("a\\"), "a\\");
    }

    #[test]
    fn octal_takes_a_leading_zero_and_at_most_three_digits() {
        assert_eq!(expand_escapes("\\0").0, vec![0]);
        assert_eq!(expand_escapes("\\01").0, vec![1]);
        assert_eq!(escaped("\\012"), "\n");
        assert_eq!(escaped("\\0123"), "S");
        // …and the fourth digit is data: 0123 is 'S', so the trailing 4 prints.
        assert_eq!(escaped("\\01234"), "S4");
    }

    #[test]
    fn hex_takes_at_most_two_digits() {
        assert_eq!(escaped("\\x4"), "\x04");
        assert_eq!(escaped("\\x41A"), "AA");
        // Above 0x7f the sequence is one raw byte, not a UTF-8 code point.
        assert_eq!(expand_escapes("\\x80").0, vec![0x80]);
    }

    /// `\c` stops output where it stands; the caller also drops the newline.
    #[test]
    fn backslash_c_truncates_the_rest() {
        let (bytes, truncated) = expand_escapes("x\\cy z");
        assert_eq!(bytes, b"x");
        assert!(truncated);
    }
}

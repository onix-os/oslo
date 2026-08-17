//! Turning what somebody typed into tokens.
//!
//! # The one hard part is that units look like names
//!
//! `2 m` is a number and a unit; `2 x` is a number and a variable; and the lexer cannot tell them
//! apart, because `m` and `x` are the same shape. So it does not try — it emits [`Token::Word`]
//! for both and lets the parser ask [`crate::units::resolve`] which one it has. That keeps the
//! decision in one place, and it is why a variable called `m` shadows the metre rather than
//! producing a parse error.
//!
//! # Numbers carry their base
//!
//! `0xff`, `0b1010` and `0o755` are the same kind of thing as `255`, and a calculator in a shell
//! is asked to convert between them constantly. The base is remembered on the token so an answer
//! can be *shown* in the base it was written in, which is what makes `0xff + 1` answer `0x100`
//! rather than `256`.
//!
//! Digit separators are allowed — `1_000_000` and `0xdead_beef` — because the alternative is
//! counting zeros.

/// How a number was written, which is also how its answer is shown unless asked otherwise.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Base {
    #[default]
    Decimal,
    Hex,
    Binary,
    Octal,
}

impl Base {
    /// The prefix this base is written with.
    pub fn prefix(self) -> &'static str {
        match self {
            Base::Decimal => "",
            Base::Hex => "0x",
            Base::Binary => "0b",
            Base::Octal => "0o",
        }
    }

    pub fn radix(self) -> u32 {
        match self {
            Base::Decimal => 10,
            Base::Hex => 16,
            Base::Binary => 2,
            Base::Octal => 8,
        }
    }
}

#[derive(Clone, PartialEq, Debug)]
pub enum Token {
    Number(f64, Base),
    /// A name: a unit, a variable, a function, a constant or a keyword. The parser decides.
    Word(String),
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Caret,
    Bang,
    Amp,
    Pipe,
    Tilde,
    Shl,
    Shr,
    LParen,
    RParen,
    Comma,
    Equals,
}

/// Split `source` into tokens, or say what could not be read.
pub fn lex(source: &str) -> Result<Vec<Token>, String> {
    let chars: Vec<char> = source.chars().collect();
    let mut out = Vec::new();
    let mut at = 0;

    while at < chars.len() {
        let c = chars[at];
        if c.is_whitespace() {
            at += 1;
            continue;
        }
        if c.is_ascii_digit() || (c == '.' && chars.get(at + 1).is_some_and(char::is_ascii_digit)) {
            out.push(number(&chars, &mut at)?);
            continue;
        }
        if is_word_start(c) {
            let start = at;
            while at < chars.len() && is_word_part(chars[at]) {
                at += 1;
            }
            out.push(Token::Word(chars[start..at].iter().collect()));
            continue;
        }
        // Two-character operators first, or `<<` reads as two comparisons of nothing.
        let pair: String = chars[at..chars.len().min(at + 2)].iter().collect();
        let token = match pair.as_str() {
            "<<" => Some(Token::Shl),
            ">>" => Some(Token::Shr),
            "**" => Some(Token::Caret),
            _ => None,
        };
        if let Some(token) = token {
            out.push(token);
            at += 2;
            continue;
        }
        let single = match c {
            '+' => Token::Plus,
            '-' | '−' => Token::Minus,
            '*' | '×' | '·' => Token::Star,
            '/' | '÷' => Token::Slash,
            '%' => Token::Percent,
            '^' => Token::Caret,
            '!' => Token::Bang,
            '&' => Token::Amp,
            '|' => Token::Pipe,
            '~' => Token::Tilde,
            '(' | '[' | '{' => Token::LParen,
            ')' | ']' | '}' => Token::RParen,
            ',' => Token::Comma,
            '=' => Token::Equals,
            // The degree sign and the micro sign are word characters as far as a unit goes, but
            // they are not `alphabetic` in a way `is_word_start` should accept generally.
            _ => return Err(format!("{c:?} is not something this understands")),
        };
        out.push(single);
        at += 1;
    }
    Ok(out)
}

/// Whether `c` can begin a name.
///
/// `°`, `µ` and `Å` are in, because they begin unit names people actually type. A digit is out, or
/// `2m` would lex as one word.
fn is_word_start(c: char) -> bool {
    c.is_alphabetic() || c == '_' || matches!(c, '°' | 'µ' | 'μ' | 'Å' | 'Ω')
}

fn is_word_part(c: char) -> bool {
    is_word_start(c) || c.is_ascii_digit()
}

/// Read one number, in whichever base it was written.
fn number(chars: &[char], at: &mut usize) -> Result<Token, String> {
    // `0x`, `0b`, `0o` — and only at the very start, so `10b` is ten followed by a name.
    if chars[*at] == '0' && *at + 1 < chars.len() {
        let base = match chars[*at + 1] {
            'x' | 'X' => Some(Base::Hex),
            'b' | 'B' => Some(Base::Binary),
            'o' | 'O' => Some(Base::Octal),
            _ => None,
        };
        if let Some(base) = base {
            *at += 2;
            let start = *at;
            let mut digits = String::new();
            while *at < chars.len() && (chars[*at].is_alphanumeric() || chars[*at] == '_') {
                if chars[*at] != '_' {
                    digits.push(chars[*at]);
                }
                *at += 1;
            }
            if digits.is_empty() {
                return Err(format!("{} has no digits after it", base.prefix()));
            }
            let value = u128::from_str_radix(&digits, base.radix()).map_err(|_| {
                format!(
                    "{:?} is not a {} number",
                    chars[start..*at].iter().collect::<String>(),
                    match base {
                        Base::Hex => "hexadecimal",
                        Base::Binary => "binary",
                        Base::Octal => "octal",
                        Base::Decimal => "decimal",
                    }
                )
            })?;
            return Ok(Token::Number(value as f64, base));
        }
    }

    let start = *at;
    let mut seen_dot = false;
    let mut text = String::new();
    while *at < chars.len() {
        let c = chars[*at];
        if c == '_' {
            *at += 1;
            continue;
        }
        if c.is_ascii_digit() {
            text.push(c);
            *at += 1;
            continue;
        }
        if c == '.' && !seen_dot {
            // A second dot ends the number: `1.2.3` is not one.
            seen_dot = true;
            text.push(c);
            *at += 1;
            continue;
        }
        // An exponent, and only when a digit really follows it. `1e6` is a million; `2e` is two
        // followed by a name, and `3 exabytes` must not lose its unit to a greedy `e`.
        if c == 'e' || c == 'E' {
            let mut probe = *at + 1;
            if matches!(chars.get(probe), Some('+' | '-')) {
                probe += 1;
            }
            if !chars.get(probe).is_some_and(char::is_ascii_digit) {
                break;
            }
            text.push('e');
            *at += 1;
            if matches!(chars.get(*at), Some('+' | '-')) {
                text.push(chars[*at]);
                *at += 1;
            }
            while *at < chars.len() && chars[*at].is_ascii_digit() {
                text.push(chars[*at]);
                *at += 1;
            }
            break;
        }
        break;
    }
    text.parse::<f64>()
        .map(|value| Token::Number(value, Base::Decimal))
        .map_err(|_| {
            format!(
                "{:?} is not a number",
                chars[start..*at].iter().collect::<String>()
            )
        })
}

#[cfg(test)]
#[path = "lex/tests.rs"]
mod tests;

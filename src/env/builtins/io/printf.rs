//! `printf` — the one output builtin that can produce exactly the bytes asked for.
//!
//! Built in rather than left to coreutils because a distro's `/bin/sh` runs before coreutils is on
//! the filesystem: an initramfs, an early boot script and a `Makefile` recipe in a stage-0 chroot
//! all call `printf` with nothing but the shell available. bash, dash and busybox all build it in
//! for the same reason. It is also the only portable way to emit a string without a trailing
//! newline, since `echo -n` is not POSIX.
//!
//! Two escape passes, which is the part that surprises people:
//!
//! * the **format** always has its backslash escapes decoded, so `printf 'a\nb'` prints two lines;
//! * an **argument** does not, unless it is consumed by `%b`, which is what `%b` is for.
//!
//! Both share `echo`'s decoder, so `\0ddd`, `\xHH` and the control escapes mean the same thing in
//! every builtin that writes bytes.

use crate::env::builtins::io::echo::expand_escapes;
use crate::env::scope::Environment;
use crate::error::Result;

/// `printf FORMAT [ARGUMENT]...`
pub fn builtin_printf(_env: &mut Environment, args: &[String]) -> Result<i32> {
    let operands = &args[1..];
    // `--` ends the options, and there are no options — but `printf -- '%s\n' x` is common enough
    // in scripts written defensively that refusing it would be a nuisance.
    let operands = match operands.first().map(String::as_str) {
        Some("--") => &operands[1..],
        _ => operands,
    };

    let Some(format) = operands.first() else {
        eprintln!("oslo: printf: usage: printf format [arguments]");
        return Ok(2);
    };
    let arguments = &operands[1..];

    let mut out: Vec<u8> = Vec::new();
    let mut status = 0;
    let mut next = 0;

    // POSIX: the format is reused until the arguments run out. `printf '%s\n' a b c` prints three
    // lines. One pass always happens, even with no arguments at all.
    loop {
        let consumed_before = next;
        match render(format, arguments, &mut next, &mut out) {
            Ok(()) => {}
            Err(code) => status = code,
        }
        // Stop unless there are arguments left *and* this pass used at least one: a format with no
        // conversions would otherwise repeat for ever.
        if next >= arguments.len() || next == consumed_before {
            break;
        }
    }

    // A write failure outranks a formatting one: if the output never arrived, the status has to
    // say so whatever the format string did.
    match super::write_stdout("printf", &out) {
        0 => Ok(status),
        failed => Ok(failed),
    }
}

/// One pass over the format. `next` indexes the next unconsumed argument.
fn render(
    format: &str,
    args: &[String],
    next: &mut usize,
    out: &mut Vec<u8>,
) -> std::result::Result<(), i32> {
    let chars: Vec<char> = format.chars().collect();
    let mut i = 0;
    let mut status = Ok(());

    while i < chars.len() {
        if chars[i] != '%' {
            // Literal text, escapes and all. Collected in one run so the decoder sees whole
            // sequences rather than a backslash at a time.
            let start = i;
            while i < chars.len() && chars[i] != '%' {
                i += 1;
            }
            let literal: String = chars[start..i].iter().collect();
            let (bytes, _) = expand_escapes(&literal);
            out.extend_from_slice(&bytes);
            continue;
        }

        // `%%` is a literal percent and consumes no argument.
        if i + 1 < chars.len() && chars[i + 1] == '%' {
            out.push(b'%');
            i += 2;
            continue;
        }

        let Some(spec) = Spec::parse(&chars, &mut i) else {
            // A trailing `%` with nothing after it. bash prints it and carries on.
            out.push(b'%');
            i = chars.len();
            continue;
        };

        // Each `*` eats an argument before the conversion's own, as C and every shell do.
        let mut sizes = Vec::new();
        for _ in 0..spec.star_count() {
            let value = args.get(*next).map(String::as_str).unwrap_or("0");
            if *next < args.len() {
                *next += 1;
            }
            // A width that is not a number is reported and treated as 0, exactly as an unreadable
            // *argument* is: the format itself is still valid, so the rest of the line is still
            // what the author asked for. bash and dash both say something here and carry on.
            sizes.push(parse_int(value).unwrap_or_else(|()| {
                eprintln!("oslo: printf: {value}: invalid number");
                status = Err(1);
                0
            }));
        }
        let spec = spec.with_sizes(&sizes);

        let arg = args.get(*next).map(String::as_str).unwrap_or("");
        if *next < args.len() {
            *next += 1;
        }
        match spec.render(arg, out) {
            Ok(()) => {}
            // A bad *argument* is reported and the format carries on, printing 0 — bash does the
            // same, because the rest of the line is still what the author asked for.
            Err(Bad::Argument(code)) => status = Err(code),
            // A conversion letter that does not exist is a bug in the format itself, so nothing
            // further in it can be trusted; bash stops there and prints no more of the line.
            Err(Bad::Format(code)) => return Err(code),
        }
    }
    status
}

/// Why a conversion failed, which decides whether the rest of the format still runs.
enum Bad {
    /// The argument could not be read as the conversion wanted. Reported; the format continues.
    Argument(i32),
    /// The conversion letter does not exist. The format is wrong, so rendering stops.
    Format(i32),
}

/// A width or a precision: written out, or `*` meaning "the next argument says".
#[derive(Clone, Copy, PartialEq, Eq)]
enum Size {
    Fixed(usize),
    /// `%*d` and `%.*f`. The argument is consumed **before** the one being converted, which is the
    /// order C and every shell use — `printf '%*d' 5 42` is width 5, value 42.
    FromArgument,
}

/// One `%` conversion: its flags, width, precision and the letter that decides the type.
struct Spec {
    flags: String,
    width: Option<Size>,
    precision: Option<Size>,
    conversion: char,
}

impl Spec {
    /// How many arguments this conversion eats before its own: one per `*`.
    fn star_count(&self) -> usize {
        usize::from(self.width == Some(Size::FromArgument))
            + usize::from(self.precision == Some(Size::FromArgument))
    }

    /// Replace each `*` with the number that was read for it, in written order.
    ///
    /// A **negative** width means left-justify at that width, exactly as a `-` flag would — C says
    /// so, and `printf '%*s' -6 hi` is how a script asks for it without knowing the sign up front.
    fn with_sizes(mut self, sizes: &[i64]) -> Self {
        let mut next = sizes.iter().copied();
        if self.width == Some(Size::FromArgument) {
            let n = next.next().unwrap_or(0);
            if n < 0 {
                self.flags.push('-');
            }
            self.width = Some(Size::Fixed(n.unsigned_abs() as usize));
        }
        if self.precision == Some(Size::FromArgument) {
            // A negative precision means "no precision given" in C, not a precision of zero.
            self.precision = match next.next().unwrap_or(0) {
                n if n < 0 => None,
                n => Some(Size::Fixed(n as usize)),
            };
        }
        self
    }

    /// The width, once every `*` has been resolved.
    fn width(&self) -> Option<usize> {
        match self.width {
            Some(Size::Fixed(n)) => Some(n),
            _ => None,
        }
    }

    /// The precision, once every `*` has been resolved.
    fn precision(&self) -> Option<usize> {
        match self.precision {
            Some(Size::Fixed(n)) => Some(n),
            _ => None,
        }
    }

    /// Parse a conversion starting at `chars[*i] == '%'`, leaving `*i` just past it.
    fn parse(chars: &[char], i: &mut usize) -> Option<Self> {
        let mut j = *i + 1;
        let mut flags = String::new();
        while j < chars.len() && matches!(chars[j], '-' | '+' | ' ' | '#' | '0') {
            flags.push(chars[j]);
            j += 1;
        }
        // `*` takes the number from an argument. Required by every shell that scripts are written
        // against — `printf '%c %*u. %s\n'` is how `select-editor` lines its menu up — and without
        // it the `*` reached the conversion letter and the whole format was refused.
        let take_size = |j: &mut usize| -> Option<Size> {
            if chars.get(*j) == Some(&'*') {
                *j += 1;
                return Some(Size::FromArgument);
            }
            take_number(chars, j).map(Size::Fixed)
        };
        let width = take_size(&mut j);
        let precision = if j < chars.len() && chars[j] == '.' {
            j += 1;
            Some(take_size(&mut j).unwrap_or(Size::Fixed(0)))
        } else {
            None
        };
        // C's length modifiers are accepted and ignored: a shell has one integer type, so `%ld`
        // and `%d` cannot differ, and scripts written against C's printf pass them. Skipping them
        // is not cosmetic — bash reads `%zb` as `%b`, so treating `z` as the conversion letter
        // made a valid format an error. `q` is deliberately absent: in a shell it is a
        // *conversion*, not a modifier.
        while j < chars.len() && matches!(chars[j], 'h' | 'l' | 'L' | 'j' | 'z' | 't') {
            j += 1;
        }
        let conversion = *chars.get(j)?;
        *i = j + 1;
        Some(Self {
            flags,
            width,
            precision,
            conversion,
        })
    }

    fn render(&self, arg: &str, out: &mut Vec<u8>) -> std::result::Result<(), Bad> {
        let mut status = Ok(());
        let body: Vec<u8> = match self.conversion {
            's' => {
                let mut s = arg.to_string();
                if let Some(p) = self.precision() {
                    s.truncate(p.min(s.len()));
                }
                s.into_bytes()
            }
            // bash's `%b`: the argument's escapes are decoded, which is the only way to get
            // `\n` out of *data* rather than out of the format.
            'b' => expand_escapes(arg).0,
            // bash's `%q`: quote the argument so the shell would read it back as this exact
            // string. What a script uses to build a command line out of untrusted data.
            'q' => shell_quote(arg).into_bytes(),
            'c' => arg
                .chars()
                .next()
                .map(String::from)
                .unwrap_or_default()
                .into_bytes(),
            'd' | 'i' => match parse_int(arg) {
                Ok(n) => n.to_string().into_bytes(),
                Err(()) => {
                    eprintln!("oslo: printf: {}: invalid number", arg);
                    status = Err(Bad::Argument(1));
                    b"0".to_vec()
                }
            },
            'u' => match parse_int(arg) {
                Ok(n) => (n as u64).to_string().into_bytes(),
                Err(()) => {
                    eprintln!("oslo: printf: {}: invalid number", arg);
                    status = Err(Bad::Argument(1));
                    b"0".to_vec()
                }
            },
            'o' | 'x' | 'X' => match parse_int(arg) {
                Ok(n) => {
                    let n = n as u64;
                    match self.conversion {
                        'o' => format!("{:o}", n),
                        'x' => format!("{:x}", n),
                        _ => format!("{:X}", n),
                    }
                    .into_bytes()
                }
                Err(()) => {
                    eprintln!("oslo: printf: {}: invalid number", arg);
                    status = Err(Bad::Argument(1));
                    b"0".to_vec()
                }
            },
            'f' | 'F' | 'e' | 'E' | 'g' | 'G' => {
                let value: f64 = arg.trim().parse().unwrap_or_else(|_| {
                    if !arg.is_empty() {
                        eprintln!("oslo: printf: {}: invalid number", arg);
                        status = Err(Bad::Argument(1));
                    }
                    0.0
                });
                let p = self.precision().unwrap_or(6);
                match self.conversion {
                    'e' => c_exponent(&format!("{:.*e}", p, value), 'e'),
                    'E' => c_exponent(&format!("{:.*E}", p, value), 'E'),
                    _ => format!("{:.*}", p, value),
                }
                .into_bytes()
            }
            other => {
                eprintln!("oslo: printf: `%{}': invalid format character", other);
                return Err(Bad::Format(1));
            }
        };

        out.extend_from_slice(&self.pad(body));
        status
    }

    /// Apply width, with `-` for left and `0` for zero-fill.
    ///
    /// Zero-fill is ignored for `%s`, as in C: padding a string with zeros produces `000abc`,
    /// which no caller means.
    fn pad(&self, body: Vec<u8>) -> Vec<u8> {
        let Some(width) = self.width() else {
            return body;
        };
        if body.len() >= width {
            return body;
        }
        let fill = if self.flags.contains('0')
            && !self.flags.contains('-')
            && !matches!(self.conversion, 's' | 'b' | 'c')
        {
            b'0'
        } else {
            b' '
        };
        let padding = vec![fill; width - body.len()];
        if self.flags.contains('-') {
            let mut out = body;
            out.extend_from_slice(&padding);
            out
        } else {
            let mut out = padding;
            out.extend_from_slice(&body);
            out
        }
    }
}

/// Quote `text` so that reading it back as shell input yields `text` exactly.
///
/// Backslash-escaping rather than wrapping in single quotes. Both are correct shell and read back
/// the same, but bash prints `a\ b` where quoting would print `'a b'`, and the differential corpus
/// compares bytes — matching the form is what lets `%q` be covered by it at all.
///
/// A control character has no backslash spelling outside `$'...'`, which is where those go; a
/// string that needs nothing is printed as itself, and an empty one as `''` so it does not vanish.
fn shell_quote(text: &str) -> String {
    if text.is_empty() {
        return "''".to_string();
    }
    if text
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "_-./=:+@,%^".contains(c))
    {
        return text.to_string();
    }
    if text.chars().any(|c| c.is_control()) {
        let mut out = String::from("$'");
        for c in text.chars() {
            match c {
                '\n' => out.push_str("\\n"),
                '\t' => out.push_str("\\t"),
                '\r' => out.push_str("\\r"),
                '\'' => out.push_str("\\'"),
                '\\' => out.push_str("\\\\"),
                c if c.is_control() => out.push_str(&format!("\\{:03o}", c as u32)),
                c => out.push(c),
            }
        }
        out.push('\'');
        return out;
    }
    let mut out = String::with_capacity(text.len() * 2);
    for c in text.chars() {
        if !c.is_ascii_alphanumeric() && !"_-./=:+@,%^".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Rewrite Rust's exponent to C's, which is what every other `printf` prints.
///
/// Rust renders `1234.5` as `1.234500e3`; C — and therefore bash, dash and coreutils — renders it
/// `1.234500e+03`, with an explicit sign and at least two digits. A script comparing `printf %e`
/// output against a recorded value would differ on every number without this.
fn c_exponent(rendered: &str, marker: char) -> String {
    let Some((mantissa, exponent)) = rendered.split_once(marker) else {
        return rendered.to_string();
    };
    let (sign, digits) = match exponent.strip_prefix('-') {
        Some(rest) => ('-', rest),
        None => ('+', exponent.strip_prefix('+').unwrap_or(exponent)),
    };
    format!("{mantissa}{marker}{sign}{:0>2}", digits)
}

fn take_number(chars: &[char], j: &mut usize) -> Option<usize> {
    let start = *j;
    while *j < chars.len() && chars[*j].is_ascii_digit() {
        *j += 1;
    }
    if *j == start {
        return None;
    }
    chars[start..*j].iter().collect::<String>().parse().ok()
}

/// Parse an integer argument the way `printf` does.
///
/// An empty argument is 0 rather than an error: a format reused past the end of its arguments
/// gets empty strings, and `printf '%d\n' 1 2` must not complain about a third that is not there.
/// A leading `'` or `"` means "the numeric value of the next character", which is POSIX and is how
/// scripts get a character's codepoint without `od`.
fn parse_int(arg: &str) -> std::result::Result<i64, ()> {
    let text = arg.trim();
    if text.is_empty() {
        return Ok(0);
    }
    if let Some(rest) = text.strip_prefix(['\'', '"']) {
        return Ok(rest.chars().next().map(|c| c as i64).unwrap_or(0));
    }
    let (sign, digits) = match text.strip_prefix('-') {
        Some(rest) => (-1, rest),
        None => (1, text.strip_prefix('+').unwrap_or(text)),
    };
    let value = if let Some(hex) = digits
        .strip_prefix("0x")
        .or_else(|| digits.strip_prefix("0X"))
    {
        i64::from_str_radix(hex, 16)
    } else if digits.len() > 1 && digits.starts_with('0') {
        i64::from_str_radix(&digits[1..], 8)
    } else {
        digits.parse()
    };
    value.map(|v| sign * v).map_err(|_| ())
}

#[cfg(test)]
mod tests {
    use super::parse_int;

    #[test]
    fn integers_are_read_in_every_base_a_shell_accepts() {
        assert_eq!(parse_int("42"), Ok(42));
        assert_eq!(parse_int("-7"), Ok(-7));
        assert_eq!(parse_int("+7"), Ok(7));
        assert_eq!(parse_int("0x1f"), Ok(31));
        assert_eq!(parse_int("010"), Ok(8));
        // An empty argument is 0: a reused format runs past its arguments and must not complain.
        assert_eq!(parse_int(""), Ok(0));
        // POSIX: a leading quote means the next character's value.
        assert_eq!(parse_int("'A"), Ok(65));
        assert_eq!(parse_int("abc"), Err(()));
    }
}

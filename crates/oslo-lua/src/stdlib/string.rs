//! The `string` library.
//!
//! Strings here are byte strings, as they are in Lua: `#"é"` is 2, and `string.sub` indexes bytes.
//! That is not a shortcut — a shell handles filenames, and a filename is a byte string that need
//! not be valid UTF-8. Treating one as text is how a program loses the ability to open a file.

use super::super::value::{Number, Value};
use super::super::{Interp, LuaError, LuaResult, ops};
use super::pattern::{self, Capture};
use super::{arg, arg_int, arg_str, module, native};

pub fn install(interp: &Interp) {
    let library = module(vec![
        ("len", native("string.len", len)),
        ("sub", native("string.sub", sub)),
        ("upper", native("string.upper", upper)),
        ("lower", native("string.lower", lower)),
        ("rep", native("string.rep", rep)),
        ("reverse", native("string.reverse", reverse)),
        // Binary packing and bytecode dumping: present and refusing, because both would need
        // machinery this evaluator does not have — a byte-level format description, and a compiler
        // that emits something to dump. Left `nil`, a script that probes for them with
        // `if string.pack then` would be right, but one that simply calls one gets
        // `attempt to call a nil value` and no idea why.
        ("pack", super::stub::missing("string.pack")),
        ("packsize", super::stub::missing("string.packsize")),
        ("unpack", super::stub::missing("string.unpack")),
        ("dump", super::stub::missing("string.dump")),
        ("byte", native("string.byte", byte)),
        ("char", native("string.char", char)),
        ("format", native("string.format", format)),
        ("find", native("string.find", find)),
        ("match", native("string.match", lua_match)),
        ("gmatch", native("string.gmatch", gmatch)),
        ("gsub", native("string.gsub", gsub)),
    ]);
    interp.set_global("string", library);
}

/// Turn a Lua index into a byte offset.
///
/// Lua indexes from 1, and a negative index counts back from the end — `s:sub(-3)` is the last
/// three bytes. Index 0 exists too, and means "before the first byte" for a start position.
fn absolute(index: i64, len: usize) -> i64 {
    if index >= 0 {
        index
    } else if (-index) as usize > len {
        0
    } else {
        len as i64 + index + 1
    }
}

fn len(_: &Interp, args: Vec<Value>) -> LuaResult<Vec<Value>> {
    Ok(vec![Value::int(arg_str(&args, 1, "len")?.len() as i64)])
}

fn sub(_: &Interp, args: Vec<Value>) -> LuaResult<Vec<Value>> {
    let s = arg_str(&args, 1, "sub")?;
    let bytes = s.as_bytes();
    let n = bytes.len();
    let mut start = absolute(arg_int(&args, 2, "sub")?, n);
    let mut end = match args.get(2) {
        Some(Value::Nil) | None => n as i64,
        _ => absolute(arg_int(&args, 3, "sub")?, n),
    };
    start = start.max(1);
    end = end.min(n as i64);
    if start > end {
        return Ok(vec![Value::str("")]);
    }
    Ok(vec![from_bytes(&bytes[start as usize - 1..end as usize])])
}

fn upper(_: &Interp, args: Vec<Value>) -> LuaResult<Vec<Value>> {
    Ok(vec![Value::str(arg_str(&args, 1, "upper")?.to_uppercase())])
}

fn lower(_: &Interp, args: Vec<Value>) -> LuaResult<Vec<Value>> {
    Ok(vec![Value::str(arg_str(&args, 1, "lower")?.to_lowercase())])
}

fn rep(_: &Interp, args: Vec<Value>) -> LuaResult<Vec<Value>> {
    let s = arg_str(&args, 1, "rep")?;
    let count = arg_int(&args, 2, "rep")?;
    if count <= 0 {
        return Ok(vec![Value::str("")]);
    }
    let separator = match args.get(2) {
        Some(Value::Nil) | None => String::new(),
        _ => arg_str(&args, 3, "rep")?,
    };
    // A cheap guard against `("x"):rep(2^40)`, which would otherwise ask for a terabyte and get
    // the allocator to abort the shell rather than raise a Lua error.
    let total = (s.len() + separator.len()).saturating_mul(count as usize);
    if total > 512 * 1024 * 1024 {
        return Err(LuaError::new("resulting string too large"));
    }
    let parts: Vec<&str> = (0..count).map(|_| s.as_str()).collect();
    Ok(vec![Value::str(parts.join(&separator))])
}

fn reverse(_: &Interp, args: Vec<Value>) -> LuaResult<Vec<Value>> {
    let mut bytes = arg_str(&args, 1, "reverse")?.into_bytes();
    bytes.reverse();
    Ok(vec![from_bytes(&bytes)])
}

fn byte(_: &Interp, args: Vec<Value>) -> LuaResult<Vec<Value>> {
    let s = arg_str(&args, 1, "byte")?;
    let bytes = s.as_bytes();
    let n = bytes.len();
    let start = match args.get(1) {
        Some(Value::Nil) | None => 1,
        _ => absolute(arg_int(&args, 2, "byte")?, n),
    };
    let end = match args.get(2) {
        Some(Value::Nil) | None => start,
        _ => absolute(arg_int(&args, 3, "byte")?, n),
    };
    let (start, end) = (start.max(1), end.min(n as i64));
    if start > end {
        return Ok(Vec::new());
    }
    Ok(bytes[start as usize - 1..end as usize]
        .iter()
        .map(|b| Value::int(*b as i64))
        .collect())
}

fn char(_: &Interp, args: Vec<Value>) -> LuaResult<Vec<Value>> {
    let mut bytes = Vec::with_capacity(args.len());
    for i in 1..=args.len() {
        let code = arg_int(&args, i, "char")?;
        if !(0..=255).contains(&code) {
            return Err(LuaError::new(format!(
                "bad argument #{i} to 'char' (value out of range)"
            )));
        }
        bytes.push(code as u8);
    }
    Ok(vec![from_bytes(&bytes)])
}

/// `string.format`, the `%` directives Lua supports.
fn format(interp: &Interp, args: Vec<Value>) -> LuaResult<Vec<Value>> {
    let spec = arg_str(&args, 1, "format")?;
    let mut out = String::new();
    let mut chars = spec.chars().peekable();
    let mut next = 2usize;

    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        if chars.peek() == Some(&'%') {
            chars.next();
            out.push('%');
            continue;
        }
        // Collect flags, width and precision as written, then dispatch on the conversion letter.
        let mut flags = String::new();
        while let Some(&f) = chars.peek() {
            if "-+ #0".contains(f) || f.is_ascii_digit() || f == '.' {
                flags.push(f);
                chars.next();
            } else {
                break;
            }
        }
        let Some(conversion) = chars.next() else {
            return Err(LuaError::new("invalid conversion to 'format'"));
        };
        let rendered = render(interp, conversion, &args, next, &flags)?;
        next += 1;
        out.push_str(&pad(&rendered, &flags, conversion));
    }
    Ok(vec![Value::str(out)])
}

/// One `%` directive's value, before width and alignment are applied.
fn render(
    interp: &Interp,
    conversion: char,
    args: &[Value],
    n: usize,
    flags: &str,
) -> LuaResult<String> {
    let precision = flags
        .split_once('.')
        .and_then(|(_, p)| p.parse::<usize>().ok());
    Ok(match conversion {
        'd' | 'i' => arg_int(args, n, "format")?.to_string(),
        'u' => (arg_int(args, n, "format")? as u64).to_string(),
        'x' => format!("{:x}", arg_int(args, n, "format")?),
        'X' => format!("{:X}", arg_int(args, n, "format")?),
        'o' => format!("{:o}", arg_int(args, n, "format")?),
        'c' => (arg_int(args, n, "format")? as u8 as char).to_string(),
        'f' | 'F' => format!("{:.*}", precision.unwrap_or(6), number(args, n)?),
        'e' => exponential(number(args, n)?, precision.unwrap_or(6), false),
        'E' => exponential(number(args, n)?, precision.unwrap_or(6), true),
        'g' | 'G' => {
            let text = format!("{}", number(args, n)?);
            if conversion == 'G' {
                text.to_uppercase()
            } else {
                text
            }
        }
        'a' | 'A' => format!("{:?}", number(args, n)?),
        's' => {
            let text = ops::tostring(interp, &arg(args, n))?;
            match precision {
                Some(p) => text.chars().take(p).collect(),
                None => text,
            }
        }
        // Lua 5.4's `%q` writes a value back as a readable literal, which is how a script dumps a
        // table it can later load.
        'q' => quote(&arg(args, n)),
        other => {
            return Err(LuaError::new(format!(
                "invalid conversion '%{other}' to 'format'"
            )));
        }
    })
}

fn number(args: &[Value], n: usize) -> LuaResult<f64> {
    arg(args, n)
        .as_number()
        .map(Number::as_float)
        .ok_or_else(|| {
            LuaError::new(format!(
                "bad argument #{n} to 'format' (number expected, got {})",
                arg(args, n).type_name()
            ))
        })
}

/// C's `%e`: a two-digit exponent, which Rust's `{:e}` does not produce.
fn exponential(value: f64, precision: usize, upper: bool) -> String {
    let text = format!("{value:.precision$e}");
    let out = match text.split_once('e') {
        Some((mantissa, exp)) => {
            let exp: i32 = exp.parse().unwrap_or(0);
            format!(
                "{mantissa}e{}{:02}",
                if exp < 0 { '-' } else { '+' },
                exp.abs()
            )
        }
        None => text,
    };
    if upper { out.to_uppercase() } else { out }
}

/// `%q` — a literal that reads back as the same value.
fn quote(value: &Value) -> String {
    let Value::Str(s) = value else {
        return value.to_display();
    };
    let mut out = String::from("\"");
    for b in s.bytes() {
        match b {
            b'"' => out.push_str("\\\""),
            b'\\' => out.push_str("\\\\"),
            b'\n' => out.push_str("\\n"),
            b'\r' => out.push_str("\\r"),
            0 => out.push_str("\\0"),
            c if c < 0x20 || c == 0x7f => out.push_str(&std::format!("\\{c}")),
            c => out.push(c as char),
        }
    }
    out.push('"');
    out
}

/// Apply width, zero-fill and left-alignment from the collected flags.
fn pad(text: &str, flags: &str, conversion: char) -> String {
    let left = flags.contains('-');
    let zero = flags.starts_with('0') || flags.contains("-0");
    let width: usize = flags
        .trim_start_matches(['-', '+', ' ', '#', '0'])
        .split('.')
        .next()
        .and_then(|w| w.parse().ok())
        .unwrap_or(0);
    let plus = flags.contains('+') && !matches!(conversion, 's' | 'q' | 'c');

    let mut body = text.to_string();
    if plus && !body.starts_with('-') {
        body.insert(0, '+');
    }
    if body.len() >= width {
        return body;
    }
    let fill = width - body.len();
    if left {
        return body + &" ".repeat(fill);
    }
    if zero && !matches!(conversion, 's' | 'q') {
        // The sign stays leftmost: `%05d` of -42 is `-0042`, not `000-42`.
        let (sign, digits) = match body.strip_prefix(['-', '+']) {
            Some(rest) => (&body[..1], rest.to_string()),
            None => ("", body),
        };
        return std::format!("{sign}{}{digits}", "0".repeat(fill));
    }
    " ".repeat(fill) + &body
}

/// A byte string as a Lua value, with invalid UTF-8 preserved rather than replaced.
///
/// `Value::Str` holds an `Rc<str>`, so a byte that is not valid UTF-8 cannot be stored as-is. The
/// lossy conversion is a known limit: `string.char(200)` does not round-trip. Fixing it means
/// changing `Value::Str` to hold bytes, which is a larger change than this library.
fn from_bytes(bytes: &[u8]) -> Value {
    Value::str(String::from_utf8_lossy(bytes))
}

/// Turn a pattern capture into the value Lua hands the script.
fn capture_value(capture: &Capture) -> Value {
    match capture {
        Capture::Text(bytes) => from_bytes(bytes),
        Capture::Position(i) => Value::int(*i as i64),
    }
}

/// The `(subject, pattern, init)` triple every matching function starts from.
fn subject(args: &[Value], function: &str) -> LuaResult<(String, String, usize)> {
    let s = arg_str(args, 1, function)?;
    let p = arg_str(args, 2, function)?;
    let init = match args.get(2) {
        Some(Value::Nil) | None => 1,
        _ => arg_int(args, 3, function)?,
    };
    let n = s.len();
    let init = absolute(init, n).max(1) as usize;
    Ok((s, p, init.saturating_sub(1)))
}

fn find(_: &Interp, args: Vec<Value>) -> LuaResult<Vec<Value>> {
    let (s, p, init) = subject(&args, "find")?;
    if init > s.len() {
        return Ok(vec![Value::Nil]);
    }

    // A fourth truthy argument turns off patterns entirely, which is how a script searches for a
    // literal string containing `-` or `.` without escaping every byte of it.
    if arg(&args, 4).truthy() {
        return Ok(match s[init..].find(&p) {
            Some(at) => vec![
                Value::int((init + at + 1) as i64),
                Value::int((init + at + p.len()) as i64),
            ],
            None => vec![Value::Nil],
        });
    }

    let found = pattern::find(s.as_bytes(), p.as_bytes(), init).map_err(LuaError::new)?;
    Ok(match found {
        Some(m) => {
            let mut out = vec![Value::int(m.start as i64 + 1), Value::int(m.end as i64)];
            out.extend(m.captures.iter().map(capture_value));
            out
        }
        None => vec![Value::Nil],
    })
}

fn lua_match(_: &Interp, args: Vec<Value>) -> LuaResult<Vec<Value>> {
    let (s, p, init) = subject(&args, "match")?;
    if init > s.len() {
        return Ok(vec![Value::Nil]);
    }
    let found = pattern::find(s.as_bytes(), p.as_bytes(), init).map_err(LuaError::new)?;
    Ok(match found {
        Some(m) => m
            .captures_or_whole(s.as_bytes())
            .iter()
            .map(capture_value)
            .collect(),
        None => vec![Value::Nil],
    })
}

fn gmatch(_: &Interp, args: Vec<Value>) -> LuaResult<Vec<Value>> {
    let s = arg_str(&args, 1, "gmatch")?;
    let p = arg_str(&args, 2, "gmatch")?;
    let cursor = std::cell::Cell::new(0usize);
    Ok(vec![native("gmatch iterator", move |_, _| {
        let from = cursor.get();
        if from > s.len() {
            return Ok(vec![Value::Nil]);
        }
        let found = pattern::find(s.as_bytes(), p.as_bytes(), from).map_err(LuaError::new)?;
        let Some(m) = found else {
            cursor.set(s.len() + 1);
            return Ok(vec![Value::Nil]);
        };
        // An empty match must still advance, or `("ab"):gmatch("x*")` never terminates.
        cursor.set(if m.end == m.start { m.end + 1 } else { m.end });
        Ok(m.captures_or_whole(s.as_bytes())
            .iter()
            .map(capture_value)
            .collect())
    })])
}

fn gsub(interp: &Interp, args: Vec<Value>) -> LuaResult<Vec<Value>> {
    let s = arg_str(&args, 1, "gsub")?;
    let p = arg_str(&args, 2, "gsub")?;
    let replacement = arg(&args, 3);
    let limit = match args.get(3) {
        Some(Value::Nil) | None => i64::MAX,
        _ => arg_int(&args, 4, "gsub")?,
    };

    let bytes = s.as_bytes();
    // **`^` anchors the whole call, not each attempt.** The matcher applies the anchor at the
    // position it is asked to start from, which is right for `find` — `("aaa"):find("^a", 2)` is
    // 2 in Lua too — and wrong here, because `gsub` walks forward. Every position then looked like
    // the beginning of the subject, so `("aaa"):gsub("^a", "X")` answered `XXX` where Lua answers
    // `Xaa`, and `("abcabc"):gsub("^abc", "-")` replaced both halves. `lstrlib.c` breaks out of its
    // loop after one attempt when the pattern is anchored; so does this.
    let anchored = p.as_bytes().first() == Some(&b'^');
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut from = 0usize;
    let mut count = 0i64;

    while count < limit && from <= bytes.len() {
        let Some(m) = pattern::find(bytes, p.as_bytes(), from).map_err(LuaError::new)? else {
            break;
        };
        out.extend_from_slice(&bytes[from..m.start]);
        let captures = m.captures_or_whole(bytes);
        let whole = &bytes[m.start..m.end];
        substitute(interp, &replacement, &captures, whole, &mut out)?;
        count += 1;
        if m.end == m.start {
            // Zero-width match: emit the byte it sat on and step past, or this loops for ever.
            if m.start < bytes.len() {
                out.push(bytes[m.start]);
            }
            from = m.start + 1;
        } else {
            from = m.end;
        }
        if anchored {
            break;
        }
    }
    if from <= bytes.len() {
        out.extend_from_slice(&bytes[from.min(bytes.len())..]);
    }
    Ok(vec![from_bytes(&out), Value::int(count)])
}

/// Apply one replacement — a string with `%n` references, a table lookup, or a function call.
fn substitute(
    interp: &Interp,
    replacement: &Value,
    captures: &[Capture],
    whole: &[u8],
    out: &mut Vec<u8>,
) -> LuaResult<()> {
    let produced = match replacement {
        Value::Str(template) => {
            let mut chars = template.bytes().peekable();
            while let Some(c) = chars.next() {
                if c != b'%' {
                    out.push(c);
                    continue;
                }
                match chars.next() {
                    // `%0` is the whole match, `%1`..`%9` the captures.
                    Some(b'0') => out.extend_from_slice(whole),
                    Some(d) if d.is_ascii_digit() => {
                        let index = (d - b'1') as usize;
                        match captures.get(index) {
                            Some(Capture::Text(t)) => out.extend_from_slice(t),
                            Some(Capture::Position(i)) => {
                                out.extend_from_slice(i.to_string().as_bytes())
                            }
                            None => {
                                return Err(LuaError::new(format!(
                                    "invalid capture index %{} in replacement string",
                                    d as char
                                )));
                            }
                        }
                    }
                    Some(other) => out.push(other),
                    None => return Err(LuaError::new("invalid use of '%' in replacement string")),
                }
            }
            return Ok(());
        }
        // A table replacement is looked up by the first capture, so `gsub(s, "%a+", words)` is a
        // dictionary substitution.
        Value::Table(t) => t.borrow().get(&capture_value(&captures[0])),
        Value::Function(_) => {
            let call_args: Vec<Value> = captures.iter().map(capture_value).collect();
            interp
                .call(replacement, call_args)?
                .into_iter()
                .next()
                .unwrap_or(Value::Nil)
        }
        Value::Number(n) => Value::str(n.to_string()),
        other => {
            return Err(LuaError::new(format!(
                "bad argument #3 to 'gsub' (string/function/table expected, got {})",
                other.type_name()
            )));
        }
    };

    match produced {
        // A table or function that answers nil or false leaves the match alone, which is what
        // makes `gsub(s, "%w+", lookup)` a safe partial substitution.
        Value::Nil | Value::Bool(false) => out.extend_from_slice(whole),
        Value::Str(s) => out.extend_from_slice(s.as_bytes()),
        Value::Number(n) => out.extend_from_slice(n.to_string().as_bytes()),
        other => {
            return Err(LuaError::new(format!(
                "invalid replacement value (a {})",
                other.type_name()
            )));
        }
    }
    Ok(())
}

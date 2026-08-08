//! The `os` library.
//!
//! Two entries are deliberately *not* what real Lua does.
//!
//! `os.execute` runs its argument through `/bin/sh`. Inside oslo that means an oslo script quietly
//! shelling out to somebody else's shell — and failing outright on a system where oslo is the only
//! one installed. It refuses and names `oslo.run{…}`, which is both safer (argv, no quoting) and
//! actually the shell you are in.
//!
//! `os.exit` in real Lua leaves immediately. Here it goes through the shell's own exit path, so
//! the EXIT trap runs and buffered output is flushed. A script that wanted the abrupt version was
//! almost certainly not asking for its own cleanup to be skipped.

use super::super::value::Value;
use super::super::{Interp, LuaError, LuaResult};
use super::{arg, arg_str, module, native, opt_str};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn install(interp: &Interp) {
    let library = module(vec![
        ("time", native("os.time", time)),
        ("clock", native("os.clock", clock)),
        ("date", native("os.date", date)),
        ("difftime", native("os.difftime", difftime)),
        ("getenv", native("os.getenv", getenv)),
        ("remove", native("os.remove", remove)),
        ("rename", native("os.rename", rename)),
        ("tmpname", native("os.tmpname", tmpname)),
        ("exit", native("os.exit", exit)),
        ("execute", native("os.execute", execute)),
        ("setlocale", native("os.setlocale", setlocale)),
    ]);
    interp.set_global("os", library);
}

fn time(_: &Interp, _: Vec<Value>) -> LuaResult<Vec<Value>> {
    // The table form — `os.time{year=…, month=…}` — needs a calendar, which is a dependency this
    // shell does not carry. `os.time()` is what scripts actually call.
    Ok(vec![Value::int(now())])
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn clock(_: &Interp, _: Vec<Value>) -> LuaResult<Vec<Value>> {
    // Real Lua reports processor time. This reports elapsed time since the process started, which
    // is what `os.clock()` is used for in practice — timing a step — and is the number a shell can
    // get without a libc call.
    Ok(vec![Value::float(elapsed())])
}

fn elapsed() -> f64 {
    use std::sync::OnceLock;
    static START: OnceLock<std::time::Instant> = OnceLock::new();
    START
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_secs_f64()
}

/// `os.date` for the formats a shell script actually uses.
///
/// Only `%Y %m %d %H %M %S %j` and `%%`, in UTC. A full `strftime` means a timezone database and a
/// locale, and getting either subtly wrong produces a plausible timestamp that is simply not the
/// time — worse than refusing. Anything else in the format string is left as written.
fn date(_: &Interp, args: Vec<Value>) -> LuaResult<Vec<Value>> {
    let format = opt_str(&args, 1, "date")?.unwrap_or_else(|| "%c".to_string());
    let stamp = match args.get(1) {
        Some(v) => v.as_number().map(|n| n.as_float() as i64).unwrap_or(now()),
        None => now(),
    };
    // `!` asks for UTC in real Lua; here everything is UTC, so it is accepted and dropped rather
    // than producing a stray `!` in the output.
    let format = format.strip_prefix('!').unwrap_or(&format);
    Ok(vec![Value::str(render(format, stamp))])
}

/// The civil date and time for a Unix timestamp, in UTC.
fn civil(stamp: i64) -> (i64, u32, u32, u32, u32, u32, u32) {
    let days = stamp.div_euclid(86_400);
    let secs = stamp.rem_euclid(86_400);
    // Howard Hinnant's days-from-civil, inverted: shifts the epoch to 1 March 0000 so that the
    // leap day lands at the end of the year and the month arithmetic has no special cases.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = if m <= 2 { y + 1 } else { y };
    let yday = day_of_year(year, m, d);
    (
        year,
        m,
        d,
        (secs / 3600) as u32,
        ((secs % 3600) / 60) as u32,
        (secs % 60) as u32,
        yday,
    )
}

fn day_of_year(year: i64, month: u32, day: u32) -> u32 {
    const CUMULATIVE: [u32; 12] = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let extra = u32::from(leap && month > 2);
    CUMULATIVE[(month - 1) as usize] + day + extra
}

fn render(format: &str, stamp: i64) -> String {
    let (year, month, day, hour, minute, second, yday) = civil(stamp);
    let mut out = String::with_capacity(format.len());
    let mut chars = format.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('Y') => out.push_str(&year.to_string()),
            Some('m') => out.push_str(&format!("{month:02}")),
            Some('d') => out.push_str(&format!("{day:02}")),
            Some('H') => out.push_str(&format!("{hour:02}")),
            Some('M') => out.push_str(&format!("{minute:02}")),
            Some('S') => out.push_str(&format!("{second:02}")),
            Some('j') => out.push_str(&format!("{yday:03}")),
            Some('c') => out.push_str(&format!(
                "{year}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02}"
            )),
            Some('%') => out.push('%'),
            // An unknown directive is copied through rather than swallowed, so a script printing
            // `%A` sees that it did not work instead of losing the character.
            Some(other) => {
                out.push('%');
                out.push(other);
            }
            None => out.push('%'),
        }
    }
    out
}

fn difftime(_: &Interp, args: Vec<Value>) -> LuaResult<Vec<Value>> {
    let later = arg(&args, 1)
        .as_number()
        .map(|n| n.as_float())
        .unwrap_or(0.0);
    let earlier = arg(&args, 2)
        .as_number()
        .map(|n| n.as_float())
        .unwrap_or(0.0);
    Ok(vec![Value::float(later - earlier)])
}

fn getenv(_: &Interp, args: Vec<Value>) -> LuaResult<Vec<Value>> {
    let name = arg_str(&args, 1, "getenv")?;
    Ok(vec![match std::env::var(&name) {
        Ok(value) => Value::str(value),
        Err(_) => Value::Nil,
    }])
}

fn remove(_: &Interp, args: Vec<Value>) -> LuaResult<Vec<Value>> {
    let path = arg_str(&args, 1, "remove")?;
    Ok(match std::fs::remove_file(&path) {
        Ok(()) => vec![Value::Bool(true)],
        Err(e) => vec![Value::Nil, Value::str(format!("{path}: {e}"))],
    })
}

fn rename(_: &Interp, args: Vec<Value>) -> LuaResult<Vec<Value>> {
    let from = arg_str(&args, 1, "rename")?;
    let to = arg_str(&args, 2, "rename")?;
    Ok(match std::fs::rename(&from, &to) {
        Ok(()) => vec![Value::Bool(true)],
        Err(e) => vec![Value::Nil, Value::str(format!("{from} -> {to}: {e}"))],
    })
}

fn tmpname(_: &Interp, _: Vec<Value>) -> LuaResult<Vec<Value>> {
    // Real Lua's `os.tmpname` returns a name and creates nothing, which is a race every security
    // guide warns about. `oslo.fs.mktemp` creates the file; this points at it rather than shipping
    // the hazard for compatibility's sake.
    Err(LuaError::new(
        "os.tmpname returns a name that anything could claim before you open it; \
         use oslo.fs.mktemp(), which creates the file",
    ))
}

fn exit(_: &Interp, args: Vec<Value>) -> LuaResult<Vec<Value>> {
    let status = match args.first() {
        // `os.exit(true)` and `os.exit(false)` are Lua's spellings of success and failure.
        Some(Value::Bool(true)) | None => 0,
        Some(Value::Bool(false)) => 1,
        Some(v) => v.as_number().map(|n| n.as_float() as i32).unwrap_or(0),
    };
    Err(LuaError::exit_request(status))
}

fn execute(_: &Interp, args: Vec<Value>) -> LuaResult<Vec<Value>> {
    // With no argument, `os.execute()` asks whether a shell is available. It is — this one.
    if args.is_empty() {
        return Ok(vec![Value::Bool(true)]);
    }
    Err(LuaError::new(
        "os.execute runs its argument through /bin/sh, which is not this shell; \
         use oslo.run{...} for an argv call or oslo.proc.exec(...) for a shell line",
    ))
}

fn setlocale(_: &Interp, _: Vec<Value>) -> LuaResult<Vec<Value>> {
    // Answering "C" is the truth: nothing here is locale-sensitive, so that is the locale in
    // force. Refusing would break scripts that set it defensively and never look again.
    Ok(vec![Value::str("C")])
}

#[cfg(test)]
mod tests {
    use super::{civil, render};

    #[test]
    fn the_epoch_and_a_leap_day_come_out_right() {
        assert_eq!(civil(0), (1970, 1, 1, 0, 0, 0, 1));
        // 2024-02-29, a leap day.
        assert_eq!(render("%Y-%m-%d", 1_709_164_800), "2024-02-29");
        assert_eq!(render("%H:%M:%S", 1_709_164_800 + 45_296), "12:34:56");
        // The day of the year has to count the leap day that came before it.
        assert_eq!(render("%j", 1_709_164_800), "060");
    }

    #[test]
    fn unknown_directives_survive_rather_than_vanishing() {
        assert_eq!(render("100%%", 0), "100%");
        assert_eq!(render("%A", 0), "%A");
    }
}

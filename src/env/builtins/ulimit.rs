//! `ulimit` — report the resource limits the shell is running under.
//!
//! Query only. Raising or lowering a limit needs `setrlimit(2)`, which this crate cannot reach:
//! `libc` is not a direct dependency and `nix`'s `resource` module is behind a Cargo feature the
//! manifest does not enable. Rather than silently accepting `ulimit -n 512` and leaving the limit
//! untouched — the failure mode that makes a script think it has headroom it does not have — the
//! set direction reports that it is unsupported and fails.
//!
//! Values are read from `/proc/self/limits`, which is the same data `getrlimit` returns, and
//! converted into the units every shell's `ulimit` reports in: 512-byte blocks for file sizes,
//! kilobytes for memory sizes, plain counts for everything else.

use crate::env::scope::Environment;
use crate::error::Result;

/// One selectable limit: its option letter, its name in `/proc/self/limits`, its description, and
/// the divisor that turns bytes into the unit `ulimit` prints.
struct Limit {
    flag: char,
    proc_name: &'static str,
    label: &'static str,
    /// 1 for a plain count or a time in seconds, 512 for a size in blocks, 1024 for a size in kB.
    divisor: u64,
}

/// One row per limit, deliberately: the table is a lookup key, a label and a unit, and wrapping
/// each row across five lines would hide that it is a table at all.
#[rustfmt::skip]
const LIMITS: &[Limit] = &[
    Limit { flag: 'c', proc_name: "Max core file size", label: "core file size (blocks)", divisor: 512 },
    Limit { flag: 'd', proc_name: "Max data size", label: "data seg size (kbytes)", divisor: 1024 },
    Limit { flag: 'e', proc_name: "Max nice priority", label: "scheduling priority", divisor: 1 },
    Limit { flag: 'f', proc_name: "Max file size", label: "file size (blocks)", divisor: 512 },
    Limit { flag: 'i', proc_name: "Max pending signals", label: "pending signals", divisor: 1 },
    Limit { flag: 'l', proc_name: "Max locked memory", label: "max locked memory (kbytes)", divisor: 1024 },
    Limit { flag: 'm', proc_name: "Max resident set", label: "max memory size (kbytes)", divisor: 1024 },
    Limit { flag: 'n', proc_name: "Max open files", label: "open files", divisor: 1 },
    Limit { flag: 'q', proc_name: "Max msgqueue size", label: "POSIX message queues (bytes)", divisor: 1 },
    Limit { flag: 'r', proc_name: "Max realtime priority", label: "real-time priority", divisor: 1 },
    Limit { flag: 's', proc_name: "Max stack size", label: "stack size (kbytes)", divisor: 1024 },
    Limit { flag: 't', proc_name: "Max cpu time", label: "cpu time (seconds)", divisor: 1 },
    Limit { flag: 'u', proc_name: "Max processes", label: "max user processes", divisor: 1 },
    Limit { flag: 'v', proc_name: "Max address space", label: "virtual memory (kbytes)", divisor: 1024 },
    Limit { flag: 'x', proc_name: "Max file locks", label: "file locks", divisor: 1 },
];

/// `ulimit [-HS] [-acdefilmnqrstuvx] [limit]`.
pub fn builtin_ulimit(_env: &mut Environment, args: &[String]) -> Result<i32> {
    let mut hard = false;
    let mut all = false;
    let mut selected: Vec<char> = Vec::new();

    let mut i = 1;
    while i < args.len() {
        let arg = &args[i];
        if arg == "--" {
            i += 1;
            break;
        }
        if arg.len() < 2 || !arg.starts_with('-') {
            break;
        }
        for c in arg[1..].chars() {
            match c {
                'H' => hard = true,
                'S' => hard = false,
                'a' => all = true,
                c if LIMITS.iter().any(|l| l.flag == c) => selected.push(c),
                other => {
                    eprintln!("rush: ulimit: -{}: invalid option", other);
                    eprintln!("ulimit: usage: ulimit [-HS] [-acdefilmnqrstuvx] [limit]");
                    return Ok(2);
                }
            }
        }
        i += 1;
    }

    if i < args.len() {
        eprintln!(
            "rush: ulimit: {}: changing a limit is not supported",
            args[i]
        );
        return Ok(1);
    }

    let Some(text) = read_limits() else {
        eprintln!("rush: ulimit: cannot read this process's resource limits");
        return Ok(1);
    };

    if all {
        for limit in LIMITS {
            match lookup(&text, limit, hard) {
                Some(value) => println!("-{}: {:<30} {}", limit.flag, limit.label, value),
                None => continue,
            }
        }
        return Ok(0);
    }

    // With no limit selected, `ulimit` reports the file-size limit, as POSIX specifies.
    if selected.is_empty() {
        selected.push('f');
    }

    let mut status = 0;
    for flag in selected {
        let limit = LIMITS
            .iter()
            .find(|l| l.flag == flag)
            .expect("validated above");
        match lookup(&text, limit, hard) {
            Some(value) => println!("{}", value),
            None => {
                eprintln!("rush: ulimit: -{}: no such limit on this system", flag);
                status = 1;
            }
        }
    }
    Ok(status)
}

fn read_limits() -> Option<String> {
    std::fs::read_to_string("/proc/self/limits").ok()
}

/// Find one limit in the text of `/proc/self/limits` and convert it to `ulimit`'s units.
fn lookup(text: &str, limit: &Limit, hard: bool) -> Option<String> {
    for line in text.lines() {
        let Some(rest) = line.strip_prefix(limit.proc_name) else {
            continue;
        };
        // The name is a prefix of the line, and "Max realtime priority" is *not* a prefix of
        // "Max realtime timeout", so a bare prefix match is unambiguous here — but the character
        // after the name must be a space, or "Max file size" would also match "Max file sizes".
        if !rest.starts_with(' ') {
            continue;
        }
        let mut fields = rest.split_whitespace();
        let soft = fields.next()?;
        let value = if hard { fields.next()? } else { soft };
        return Some(match value {
            "unlimited" => "unlimited".to_string(),
            n => (n.parse::<u64>().ok()? / limit.divisor).to_string(),
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{LIMITS, builtin_ulimit, lookup, read_limits};
    use crate::env::Environment;

    const SAMPLE: &str = "\
Limit                     Soft Limit           Hard Limit           Units
Max cpu time              unlimited            unlimited            seconds
Max file size             unlimited            unlimited            bytes
Max stack size            8388608              unlimited            bytes
Max core file size        0                    unlimited            bytes
Max open files            1024                 1048576              files
";

    fn limit_for(flag: char) -> &'static super::Limit {
        LIMITS.iter().find(|l| l.flag == flag).expect("known flag")
    }

    /// The soft limit is the default, and the hard one is what `-H` asks for.
    #[test]
    fn soft_and_hard_limits_are_read_separately() {
        assert_eq!(
            lookup(SAMPLE, limit_for('n'), false).as_deref(),
            Some("1024")
        );
        assert_eq!(
            lookup(SAMPLE, limit_for('n'), true).as_deref(),
            Some("1048576")
        );
    }

    /// Sizes are reported in the units every shell's `ulimit` uses, not in bytes: kilobytes for
    /// memory, 512-byte blocks for files.
    #[test]
    fn sizes_are_converted_to_the_units_shells_report() {
        assert_eq!(
            lookup(SAMPLE, limit_for('s'), false).as_deref(),
            Some("8192")
        );
        assert_eq!(lookup(SAMPLE, limit_for('c'), false).as_deref(), Some("0"));
        assert_eq!(
            lookup(SAMPLE, limit_for('t'), false).as_deref(),
            Some("unlimited")
        );
    }

    #[test]
    fn a_limit_this_kernel_does_not_report_is_absent() {
        assert_eq!(lookup(SAMPLE, limit_for('x'), false), None);
    }

    /// Accepting a value and doing nothing would tell a script it has headroom it does not have.
    #[test]
    fn setting_a_limit_is_refused_rather_than_ignored() {
        let mut env = Environment::new();
        let args = vec!["ulimit".to_string(), "-n".to_string(), "512".to_string()];
        assert_eq!(builtin_ulimit(&mut env, &args).unwrap(), 1);
    }

    #[test]
    #[cfg(target_os = "linux")]
    fn the_running_process_reports_its_own_open_file_limit() {
        let text = read_limits().expect("/proc/self/limits");
        assert!(lookup(&text, limit_for('n'), false).is_some());
    }
}

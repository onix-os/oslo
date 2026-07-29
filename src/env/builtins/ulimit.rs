//! `ulimit` — report and change the resource limits the shell is running under.
//!
//! Both directions go through `getrlimit(2)`/`setrlimit(2)`. Reading used to parse
//! `/proc/self/limits` instead — the same data, but only on Linux, so every `ulimit` query on
//! macOS reported "cannot read this process's resource limits" while `ulimit -n 128` beside it
//! silently succeeded. Values are converted into the units every shell's `ulimit` reports in:
//! 512-byte blocks for file sizes, kilobytes for memory sizes, plain counts for everything else.
//!
//! Setting goes through `setrlimit(2)`, reached via `nix`'s `resource` feature. Not every limit
//! exists on every system — `RLIMIT_NICE` and `RLIMIT_MSGQUEUE` are Linux's, not POSIX's — so
//! [`resource_for`] answers `None` where the platform has no such limit and the builtin says so
//! rather than pretending the limit was applied. That distinction is the whole reason this
//! builtin refused the set direction outright before it could implement it: a script told it has
//! headroom it does not have will happily open the files it cannot open.

use crate::env::scope::Environment;
use crate::error::Result;
use nix::sys::resource::{RLIM_INFINITY, Resource, getrlimit, rlim_t, setrlimit};

/// One selectable limit: its option letter, its description, and the divisor that turns the
/// kernel's units into the unit `ulimit` prints.
struct Limit {
    flag: char,
    label: &'static str,
    /// 1 for a plain count or a time in seconds, 512 for a size in blocks, 1024 for a size in kB.
    divisor: u64,
}

/// One row per limit, deliberately: the table is a lookup key, a label and a unit, and wrapping
/// each row across five lines would hide that it is a table at all.
#[rustfmt::skip]
const LIMITS: &[Limit] = &[
    Limit { flag: 'c', label: "core file size (blocks)", divisor: 512 },
    Limit { flag: 'd', label: "data seg size (kbytes)", divisor: 1024 },
    Limit { flag: 'e', label: "scheduling priority", divisor: 1 },
    Limit { flag: 'f', label: "file size (blocks)", divisor: 512 },
    Limit { flag: 'i', label: "pending signals", divisor: 1 },
    Limit { flag: 'l', label: "max locked memory (kbytes)", divisor: 1024 },
    Limit { flag: 'm', label: "max memory size (kbytes)", divisor: 1024 },
    Limit { flag: 'n', label: "open files", divisor: 1 },
    Limit { flag: 'q', label: "POSIX message queues (bytes)", divisor: 1 },
    Limit { flag: 'r', label: "real-time priority", divisor: 1 },
    Limit { flag: 's', label: "stack size (kbytes)", divisor: 1024 },
    Limit { flag: 't', label: "cpu time (seconds)", divisor: 1 },
    Limit { flag: 'u', label: "max user processes", divisor: 1 },
    Limit { flag: 'v', label: "virtual memory (kbytes)", divisor: 1024 },
    Limit { flag: 'x', label: "file locks", divisor: 1 },
];

/// Which of the two limits a `-H`/`-S` run selected.
///
/// Three states, not a `bool`: reporting needs to know *which* limit to print, and setting needs
/// to know that neither flag was given, because a bare `ulimit -n 100` moves the hard limit down
/// with the soft one and `ulimit -S -n 100` does not.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Which {
    Soft,
    Hard,
    Both,
}

impl Which {
    /// Whether a report should print the hard limit. `Both` cannot occur while reporting — no
    /// flag means the soft limit, which is the one a process is actually running under.
    fn reports_hard(self) -> bool {
        self == Which::Hard
    }
}

/// `ulimit [-HS] [-acdefilmnqrstuvx] [limit]`.
pub fn builtin_ulimit(_env: &mut Environment, args: &[String]) -> Result<i32> {
    let mut which = Which::Both;
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
                'H' => which = Which::Hard,
                'S' => which = Which::Soft,
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

    // With no limit selected, `ulimit` means the file-size limit, as POSIX specifies — for
    // setting as much as for reporting.
    if selected.is_empty() {
        selected.push('f');
    }

    // An operand is a *new* limit, and setting produces no output at all: `ulimit -n 512` is
    // silent, and the `ulimit -n` after it prints 512.
    if i < args.len() {
        let mut status = 0;
        for flag in &selected {
            status |= set_one(*flag, &args[i], which);
        }
        return Ok(status);
    }

    let hard = which.reports_hard();

    // `-a` reports what this system actually has: a limit the platform does not define is skipped
    // rather than printed as an error, so the listing stays a listing.
    if all {
        for limit in LIMITS {
            match report(limit, hard) {
                Some(value) => println!("-{}: {:<30} {}", limit.flag, limit.label, value),
                None => continue,
            }
        }
        return Ok(0);
    }

    let mut status = 0;
    for flag in selected {
        let limit = LIMITS
            .iter()
            .find(|l| l.flag == flag)
            .expect("validated above");
        match report(limit, hard) {
            Some(value) => println!("{}", value),
            None => {
                eprintln!("rush: ulimit: -{}: no such limit on this system", flag);
                status = 1;
            }
        }
    }
    Ok(status)
}

/// The `getrlimit`/`setrlimit` resource a flag selects, or `None` where this system has no such
/// limit.
///
/// Six of these are Linux's own. Naming them unconditionally would not compile on macOS, which
/// the release matrix builds; answering `None` there is what turns "this system has no such
/// limit" into a diagnostic instead of a lie.
#[rustfmt::skip]
fn resource_for(flag: char) -> Option<Resource> {
    Some(match flag {
        'c' => Resource::RLIMIT_CORE,
        'd' => Resource::RLIMIT_DATA,
        'f' => Resource::RLIMIT_FSIZE,
        'n' => Resource::RLIMIT_NOFILE,
        's' => Resource::RLIMIT_STACK,
        't' => Resource::RLIMIT_CPU,
        #[cfg(not(any(target_os = "freebsd", target_os = "netbsd", target_os = "openbsd")))]
        'v' => Resource::RLIMIT_AS,
        #[cfg(any(target_os = "linux", target_os = "android"))]
        'e' => Resource::RLIMIT_NICE,
        #[cfg(any(target_os = "linux", target_os = "android"))]
        'i' => Resource::RLIMIT_SIGPENDING,
        #[cfg(any(target_os = "linux", target_os = "android"))]
        'l' => Resource::RLIMIT_MEMLOCK,
        #[cfg(any(target_os = "linux", target_os = "android"))]
        'm' => Resource::RLIMIT_RSS,
        #[cfg(any(target_os = "linux", target_os = "android"))]
        'q' => Resource::RLIMIT_MSGQUEUE,
        #[cfg(any(target_os = "linux", target_os = "android"))]
        'r' => Resource::RLIMIT_RTPRIO,
        #[cfg(any(target_os = "linux", target_os = "android"))]
        'u' => Resource::RLIMIT_NPROC,
        #[cfg(any(target_os = "linux", target_os = "android"))]
        'x' => Resource::RLIMIT_LOCKS,
        _ => return None,
    })
}

/// What a `ulimit` operand means, in the units the flag is reported in.
///
/// `hard` and `soft` are not numbers and must not be parsed as any: they name whichever value
/// the limit currently holds, which is how `ulimit -n hard` raises the soft limit as far as it
/// is allowed to go.
enum Requested {
    Value(rlim_t),
    Unlimited,
    Hard,
    Soft,
}

fn parse_operand(operand: &str, divisor: u64) -> Option<Requested> {
    match operand {
        "unlimited" => Some(Requested::Unlimited),
        "hard" => Some(Requested::Hard),
        "soft" => Some(Requested::Soft),
        // Multiplied into bytes, since the flag reports blocks or kilobytes. A value so large
        // that the multiplication would wrap is not a limit any kernel can hold.
        n => n
            .parse::<u64>()
            .ok()?
            .checked_mul(divisor)
            .map(|v| Requested::Value(v as rlim_t)),
    }
}

/// Apply one `ulimit -FLAG value`. Returns the exit status, having reported any failure.
fn set_one(flag: char, operand: &str, which: Which) -> i32 {
    let limit = LIMITS
        .iter()
        .find(|l| l.flag == flag)
        .expect("the option run only accepts flags from the table");
    let Some(resource) = resource_for(flag) else {
        eprintln!("rush: ulimit: -{}: no such limit on this system", flag);
        return 1;
    };
    let Ok((soft, hard)) = getrlimit(resource) else {
        eprintln!("rush: ulimit: -{}: cannot read the current limit", flag);
        return 1;
    };
    let Some(requested) = parse_operand(operand, limit.divisor) else {
        eprintln!("rush: ulimit: {}: invalid number", operand);
        return 1;
    };
    let value = match requested {
        Requested::Value(v) => v,
        // `ulimit -S -n unlimited` means "as high as I am allowed", not "past the ceiling":
        // bash raises the soft limit to the hard one and exits 0, where a literal
        // `RLIM_INFINITY` would be `EINVAL` whenever the hard limit is finite. Raising the
        // *hard* limit is a different request and is still allowed to fail.
        Requested::Unlimited if which == Which::Soft => hard,
        Requested::Unlimited => RLIM_INFINITY,
        Requested::Hard => hard,
        Requested::Soft => soft,
    };

    // Whichever half was not selected keeps the value it had. The kernel, not this code, decides
    // whether the result is legal: lowering a hard limit below the soft one is `EINVAL` and
    // raising a hard limit without privilege is `EPERM`, and both are worth reporting exactly.
    let (new_soft, new_hard) = match which {
        Which::Soft => (value, hard),
        Which::Hard => (soft, value),
        Which::Both => (value, value),
    };
    match setrlimit(resource, new_soft, new_hard) {
        Ok(()) => 0,
        Err(errno) => {
            eprintln!(
                "rush: ulimit: {}: cannot modify limit: {}",
                limit.label, errno
            );
            1
        }
    }
}

/// Read one limit and convert it to `ulimit`'s units, or `None` where the platform has no such
/// limit — which is the same answer [`resource_for`] gives the set direction, so a flag cannot be
/// reportable and unsettable or the reverse.
fn report(limit: &Limit, hard: bool) -> Option<String> {
    let resource = resource_for(limit.flag)?;
    let (soft_value, hard_value) = getrlimit(resource).ok()?;
    let value = if hard { hard_value } else { soft_value };
    Some(format_limit(value, limit.divisor))
}

/// Render one raw limit in the unit its flag reports.
///
/// Split from [`report`] so the conversion can be tested without `setrlimit`: the limits belong to
/// the whole test process, and a test that moves them races every test running beside it.
fn format_limit(value: rlim_t, divisor: u64) -> String {
    if value == RLIM_INFINITY {
        return "unlimited".to_string();
    }
    // Truncating division, matching bash: a limit that is not a whole number of blocks is
    // reported as the number of whole blocks it covers. `rlim_t` is `u64` on every target in the
    // matrix, so this divides without a conversion — clippy rejects one as useless either way.
    (value / divisor).to_string()
}

#[cfg(test)]
mod tests {
    use super::{LIMITS, builtin_ulimit, format_limit, report, resource_for};
    use crate::env::Environment;
    use nix::sys::resource::RLIM_INFINITY;

    fn limit_for(flag: char) -> &'static super::Limit {
        LIMITS.iter().find(|l| l.flag == flag).expect("known flag")
    }

    /// Sizes are reported in the units every shell's `ulimit` uses, not in bytes: kilobytes for
    /// memory, 512-byte blocks for files.
    #[test]
    fn sizes_are_converted_to_the_units_shells_report() {
        assert_eq!(format_limit(8_388_608, 1024), "8192");
        assert_eq!(format_limit(4096 * 512, 512), "4096");
        assert_eq!(format_limit(1024, 1), "1024");
        assert_eq!(format_limit(0, 512), "0");
        assert_eq!(format_limit(RLIM_INFINITY, 1024), "unlimited");
    }

    /// The six POSIX limits have to exist everywhere rush builds. This is what stops a query path
    /// from being silently unavailable on one platform — which is exactly how the `/proc`-based
    /// reader this replaced went unnoticed until CI ran on macOS.
    #[test]
    fn the_posix_limits_are_readable_on_every_platform() {
        for flag in ['c', 'd', 'f', 'n', 's', 't'] {
            assert!(
                resource_for(flag).is_some(),
                "-{flag} has no resource on this platform"
            );
            assert!(
                report(limit_for(flag), false).is_some(),
                "-{flag} could not be read on this platform"
            );
        }
    }

    /// A limit really is applied: lower the soft core-file limit and read it back through the
    /// same path a script would. Restored afterwards, because this is the test process's own
    /// limit and every later test inherits it.
    #[test]
    fn setting_a_limit_changes_what_the_query_reports() {
        use nix::sys::resource::{Resource, getrlimit, setrlimit};
        let (soft, hard) = getrlimit(Resource::RLIMIT_CORE).expect("core limit");

        let mut env = Environment::new();
        let args = vec!["ulimit".to_string(), "-Sc".to_string(), "0".to_string()];
        assert_eq!(builtin_ulimit(&mut env, &args).unwrap(), 0);
        assert_eq!(report(limit_for('c'), false).as_deref(), Some("0"));

        setrlimit(Resource::RLIMIT_CORE, soft, hard).expect("restore");
    }

    /// A value that is not a number is refused rather than rounded to zero, which would silently
    /// forbid what the author meant to allow.
    #[test]
    fn a_non_numeric_limit_is_refused() {
        let mut env = Environment::new();
        let args = vec!["ulimit".to_string(), "-c".to_string(), "abc".to_string()];
        assert_eq!(builtin_ulimit(&mut env, &args).unwrap(), 1);
    }

    /// `hard`, `soft` and `unlimited` name limits; everything else is a count in the flag's own
    /// unit, so a file-size operand is multiplied into bytes before it reaches the kernel.
    #[test]
    fn operands_are_named_limits_or_counts_in_the_flags_unit() {
        use super::{Requested, parse_operand};
        assert!(matches!(
            parse_operand("unlimited", 1),
            Some(Requested::Unlimited)
        ));
        assert!(matches!(parse_operand("hard", 512), Some(Requested::Hard)));
        assert!(matches!(parse_operand("soft", 512), Some(Requested::Soft)));
        assert!(matches!(
            parse_operand("2", 512),
            Some(Requested::Value(1024))
        ));
        assert!(parse_operand("abc", 1).is_none());
        assert!(parse_operand("-1", 1).is_none());
        assert!(
            parse_operand(&u64::MAX.to_string(), 1024).is_none(),
            "a value that overflows its own unit is not a limit"
        );
    }
}

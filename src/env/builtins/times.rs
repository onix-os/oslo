//! `times` — accumulated user and system time for the shell and its children.
//!
//! Two lines, `user system`: the shell's own times first, its waited-for children's second. The
//! numbers come from `/proc/self/stat` rather than from `times(2)`, because the `libc` crate is
//! not a direct dependency and `nix` only exposes the call behind a feature this crate does not
//! enable. On a system without `/proc` the builtin reports zeroes rather than failing — a wrong
//! number in a diagnostic is better than a script aborting on a builtin that cannot fail in any
//! other shell.

use crate::env::scope::Environment;
use crate::error::Result;

/// `USER_HZ`: the unit `/proc/[pid]/stat` reports CPU times in.
///
/// Fixed at 100 by the kernel's ABI for these fields regardless of the configured tick rate, so
/// it does not need `sysconf(_SC_CLK_TCK)` to be readable from here.
const USER_HZ: f64 = 100.0;

/// `times` — print the shell's and its children's CPU time.
pub fn builtin_times(_env: &mut Environment, _args: &[String]) -> Result<i32> {
    let t = read_cpu_times().unwrap_or([0; 4]);
    println!("{} {}", format_ticks(t[0]), format_ticks(t[1]));
    println!("{} {}", format_ticks(t[2]), format_ticks(t[3]));
    Ok(0)
}

/// `[utime, stime, cutime, cstime]` in clock ticks, from `/proc/self/stat`.
fn read_cpu_times() -> Option<[u64; 4]> {
    let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
    parse_stat(&stat)
}

/// Pull the four time fields out of a `/proc/[pid]/stat` line.
///
/// Field 2 is the executable name in parentheses and may itself contain spaces and parentheses,
/// so the split has to start after the *last* `)`, not at the second whitespace-separated token.
fn parse_stat(stat: &str) -> Option<[u64; 4]> {
    let tail = &stat[stat.rfind(')')? + 1..];
    let fields: Vec<&str> = tail.split_whitespace().collect();
    // `tail` starts at field 3, so field 14 (utime) is index 11.
    let at = |i: usize| fields.get(i).and_then(|f| f.parse::<u64>().ok());
    Some([at(11)?, at(12)?, at(13)?, at(14)?])
}

/// Render clock ticks the way every shell's `times` does: `<minutes>m<seconds>s`.
fn format_ticks(ticks: u64) -> String {
    let seconds = ticks as f64 / USER_HZ;
    let minutes = (seconds / 60.0) as u64;
    format!("{}m{:.3}s", minutes, seconds - (minutes * 60) as f64)
}

#[cfg(test)]
mod tests {
    use super::{format_ticks, parse_stat, read_cpu_times};

    #[test]
    fn ticks_render_as_minutes_and_seconds() {
        assert_eq!(format_ticks(0), "0m0.000s");
        assert_eq!(format_ticks(1), "0m0.010s");
        assert_eq!(format_ticks(6_000), "1m0.000s");
        assert_eq!(format_ticks(6_150), "1m1.500s");
    }

    /// The command name is attacker-controlled — it is the basename of whatever `exec`'d the
    /// process — so a name full of spaces and parentheses must not shift the field indices.
    #[test]
    fn a_command_name_with_spaces_does_not_shift_the_fields() {
        let stat = "42 (odd ) name) S 1 42 42 0 -1 0 0 0 0 0 11 22 33 44 20 0 1 0";
        assert_eq!(parse_stat(stat), Some([11, 22, 33, 44]));
    }

    #[test]
    fn a_malformed_line_is_refused_rather_than_guessed_at() {
        assert_eq!(parse_stat("no parenthesis here"), None);
        assert_eq!(parse_stat("42 (sh) S 1 2 3"), None);
    }

    /// The real file, on the platform this shell targets.
    #[test]
    #[cfg(target_os = "linux")]
    fn the_running_process_reports_its_own_times() {
        assert!(read_cpu_times().is_some());
    }
}

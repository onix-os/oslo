//! Signal names and numbers, in both directions.
//!
//! nix's `Signal` only round-trips the canonical `SIGxxx` spelling and only covers the standard
//! signals. Every shell also accepts the bare name (`kill -HUP`), the number (`kill -1`) and,
//! on Linux, the realtime names (`kill -RTMIN+3`) — so the mapping lives here instead of being
//! open-coded at the one call site that happened to need it.

use nix::sys::signal::Signal;
use std::str::FromStr;

/// Signal numbers a `kill -l` listing walks. 32 and 33 are reserved by glibc's threading
/// implementation and have no name; `Signal::try_from` rejects them, so they drop out.
const STANDARD_RANGE: std::ops::RangeInclusive<i32> = 1..=31;

/// Resolve a `kill` signal spec — a name, a `SIG`-prefixed name, or a number.
///
/// Returns `None` for anything that does not name a signal this system can deliver. `0` resolves
/// to `Some(0)`: it is not a signal but it is a valid spec, and it means "probe, deliver nothing".
pub fn parse_spec(spec: &str) -> Option<i32> {
    if let Ok(num) = spec.parse::<i32>() {
        return (num == 0 || name_from_number(num).is_some()).then_some(num);
    }
    number_from_name(spec)
}

/// Number for a signal name, with or without the `SIG` prefix, in any case.
pub fn number_from_name(name: &str) -> Option<i32> {
    let upper = name.to_uppercase();
    let bare = upper.strip_prefix("SIG").unwrap_or(&upper);
    if bare.is_empty() {
        return None;
    }
    if let Some(rt) = realtime_from_name(bare) {
        return Some(rt);
    }
    Signal::from_str(&format!("SIG{bare}"))
        .ok()
        .map(|s| s as i32)
}

/// Name (without the `SIG` prefix) for a signal number.
pub fn name_from_number(num: i32) -> Option<String> {
    if let Some(rt) = realtime_name(num) {
        return Some(rt);
    }
    Signal::try_from(num)
        .ok()
        .map(|s| s.as_str().trim_start_matches("SIG").to_string())
}

/// Every signal this system names, in number order — the operand-less `kill -l` listing.
pub fn all() -> Vec<(i32, String)> {
    let mut out: Vec<(i32, String)> = STANDARD_RANGE
        .filter_map(|n| name_from_number(n).map(|name| (n, name)))
        .collect();
    out.extend(realtime_range().filter_map(|n| realtime_name(n).map(|name| (n, name))));
    out
}

/// Realtime signals exist on Linux only; elsewhere the range is empty and the names never match.
#[cfg(any(target_os = "linux", target_os = "android"))]
fn realtime_range() -> std::ops::RangeInclusive<i32> {
    nix::libc::SIGRTMIN()..=nix::libc::SIGRTMAX()
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
fn realtime_range() -> std::ops::RangeInclusive<i32> {
    #[allow(clippy::reversed_empty_ranges)]
    {
        1..=0
    }
}

/// `RTMIN`, `RTMAX`, `RTMIN+n` and `RTMAX-n`, the spelling `kill -l` prints and accepts back.
fn realtime_from_name(bare: &str) -> Option<i32> {
    let range = realtime_range();
    if range.is_empty() {
        return None;
    }
    let num = if let Some(rest) = bare.strip_prefix("RTMIN") {
        range.start().checked_add(offset(rest, '+')?)?
    } else if let Some(rest) = bare.strip_prefix("RTMAX") {
        range.end().checked_sub(offset(rest, '-')?)?
    } else {
        return None;
    };
    range.contains(&num).then_some(num)
}

/// The `+n` / `-n` tail of a realtime name. An empty tail is offset zero; anything that is not
/// the expected sign followed by a number is not a signal name at all.
fn offset(tail: &str, sign: char) -> Option<i32> {
    if tail.is_empty() {
        return Some(0);
    }
    tail.strip_prefix(sign)?
        .parse::<i32>()
        .ok()
        .filter(|n| *n >= 0)
}

/// Name a realtime signal the way bash does: the low half counts up from `RTMIN`, the high half
/// counts back from `RTMAX`, so the two shells' `kill -l` output agrees.
fn realtime_name(num: i32) -> Option<String> {
    let range = realtime_range();
    if !range.contains(&num) {
        return None;
    }
    let (min, max) = (*range.start(), *range.end());
    let from_min = num - min;
    if from_min <= (max - min) / 2 {
        Some(if from_min == 0 {
            "RTMIN".to_string()
        } else {
            format!("RTMIN+{from_min}")
        })
    } else {
        let from_max = max - num;
        Some(if from_max == 0 {
            "RTMAX".to_string()
        } else {
            format!("RTMAX-{from_max}")
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_resolve_with_and_without_the_sig_prefix() {
        // The bug this file exists for: nix wants SIGHUP, users write -HUP and -hup.
        assert_eq!(number_from_name("HUP"), Some(1));
        assert_eq!(number_from_name("SIGHUP"), Some(1));
        assert_eq!(number_from_name("hup"), Some(1));
        assert_eq!(number_from_name("sigusr1"), number_from_name("USR1"));
        assert_eq!(number_from_name("NOSUCHSIG"), None);
        assert_eq!(number_from_name(""), None);
        assert_eq!(number_from_name("SIG"), None);
    }

    #[test]
    fn numbers_round_trip_to_names() {
        assert_eq!(name_from_number(9).as_deref(), Some("KILL"));
        assert_eq!(name_from_number(15).as_deref(), Some("TERM"));
        assert_eq!(name_from_number(0), None);
        assert_eq!(name_from_number(4096), None);
        for (num, name) in all() {
            assert_eq!(number_from_name(&name), Some(num), "{name} -> {num}");
        }
    }

    #[test]
    fn zero_is_a_valid_spec_but_not_a_signal() {
        assert_eq!(parse_spec("0"), Some(0));
        assert_eq!(parse_spec("9"), Some(9));
        assert_eq!(parse_spec("99"), None);
        assert_eq!(parse_spec("-1"), None);
        assert_eq!(parse_spec("KILL"), Some(9));
        assert_eq!(parse_spec("NOSUCHSIG"), None);
    }

    #[cfg(any(target_os = "linux", target_os = "android"))]
    #[test]
    fn realtime_names_match_the_kernel_range() {
        let min = nix::libc::SIGRTMIN();
        let max = nix::libc::SIGRTMAX();
        assert_eq!(number_from_name("RTMIN"), Some(min));
        assert_eq!(number_from_name("RTMAX"), Some(max));
        assert_eq!(number_from_name("SIGRTMIN+1"), Some(min + 1));
        assert_eq!(name_from_number(max).as_deref(), Some("RTMAX"));
        assert_eq!(number_from_name("RTMIN+999"), None);
        assert_eq!(number_from_name("RTMIN-1"), None);
    }
}

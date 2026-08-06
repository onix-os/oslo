//! The report the `time` keyword leaves behind.
//!
//! Split out of [`super`] because it is the only part of pipeline evaluation that asks the kernel
//! about resource usage, and because the report is purely a *side* channel: a timed pipeline runs,
//! writes and reports its status exactly as an untimed one does. The single rule this module
//! exists to enforce is that the three lines go to **stderr**, never stdout — `x=$(time cmd)` has
//! to capture `cmd`'s output alone, and a `time` that polluted a command substitution would be
//! worse than the silent drop it replaced (R8.7).

use nix::libc;
use std::time::{Duration, Instant};

/// CPU time consumed so far by this shell and by every child it has already reaped.
///
/// Both halves are needed. A builtin (`time read x`) burns only the shell's own time; an external
/// command (`time sleep 1`) only its children's; a mixed pipeline splits between them. Reporting
/// either alone makes half the pipelines a script can write look free.
///
/// `RUSAGE_CHILDREN` is cumulative over the shell's whole life, which is why callers take a
/// snapshot before the pipeline and subtract — see [`Timer`].
#[derive(Clone, Copy)]
struct CpuTimes {
    user: Duration,
    sys: Duration,
}

impl CpuTimes {
    fn now() -> Self {
        let (self_user, self_sys) = getrusage(libc::RUSAGE_SELF);
        let (child_user, child_sys) = getrusage(libc::RUSAGE_CHILDREN);
        Self {
            user: self_user + child_user,
            sys: self_sys + child_sys,
        }
    }
}

/// The user and system time `who` reports, or zero if the kernel refuses to say.
///
/// Zero rather than an error: `time` is a report, not a command, and failing the pipeline because
/// its accounting could not be read would turn a cosmetic problem into a broken script.
fn getrusage(who: libc::c_int) -> (Duration, Duration) {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();
    // SAFETY: `getrusage` writes only through the pointer it is given, which points at a
    // correctly sized and aligned `rusage` owned by this frame.
    let rc = unsafe { libc::getrusage(who, usage.as_mut_ptr()) };
    if rc != 0 {
        return (Duration::ZERO, Duration::ZERO);
    }
    // SAFETY: `getrusage` returned 0, so it filled the struct.
    let usage = unsafe { usage.assume_init() };
    (
        timeval_to_duration(usage.ru_utime),
        timeval_to_duration(usage.ru_stime),
    )
}

/// A `timeval` as a [`Duration`], clamping rather than trusting the sign.
///
/// `Duration` cannot hold a negative span and `time_t`/`suseconds_t` are signed; a hostile or
/// buggy value must not panic the shell in the middle of reporting a timing.
fn timeval_to_duration(tv: libc::timeval) -> Duration {
    let secs = tv.tv_sec.max(0) as u64;
    let micros = tv.tv_usec.clamp(0, 999_999) as u32;
    Duration::new(secs, micros * 1_000)
}

/// A measurement in progress. [`Timer::report`] stops it and writes the three lines.
pub(super) struct Timer {
    wall: Instant,
    cpu: CpuTimes,
}

impl Timer {
    pub(super) fn start() -> Self {
        Self {
            wall: Instant::now(),
            cpu: CpuTimes::now(),
        }
    }

    /// Write `real`, `user` and `sys` for everything that happened since [`Timer::start`].
    pub(super) fn report(self) {
        let real = self.wall.elapsed();
        let end = CpuTimes::now();
        // Saturating, not plain subtraction: `getrusage` is monotonic, but a failed call reports
        // zero, and a zero "after" would otherwise wrap into a 584-year `user`.
        let user = end.user.saturating_sub(self.cpu.user);
        let sys = end.sys.saturating_sub(self.cpu.sys);
        write_report(real, user, sys);
        // `on-time-report`: only a `time`-prefixed pipeline reaches here, which is what separates
        // this from `post-cmd` — that one fires for everything and carries wall-clock only. These
        // are the three clocks, and they were asked for.
        crate::lua::engine::fire_at_here(
            crate::lua::api::hooks::at::TIME_REPORT,
            &[
                ("real_ms", &real.as_millis().to_string()),
                ("user_ms", &user.as_millis().to_string()),
                ("sys_ms", &sys.as_millis().to_string()),
            ],
        );
    }
}

/// Emit bash's shape: a blank line, then one tab-separated line per clock.
///
/// One `eprint!` rather than four, so the block cannot be split by another writer — a background
/// job sharing this stderr is exactly the case where a half-printed report is unreadable.
fn write_report(real: Duration, user: Duration, sys: Duration) {
    eprint!(
        "\nreal\t{}\nuser\t{}\nsys\t{}\n",
        format_elapsed(real),
        format_elapsed(user),
        format_elapsed(sys)
    );
}

/// bash's default `TIMEFORMAT` field: whole minutes, then seconds to the millisecond.
///
/// Computed in integer milliseconds rather than through `f64` seconds so that a value just under a
/// minute cannot round up into the nonsense `0m60.000s`.
fn format_elapsed(d: Duration) -> String {
    let millis = d.as_millis();
    let minutes = millis / 60_000;
    let rest = millis % 60_000;
    format!("{}m{}.{:03}s", minutes, rest / 1_000, rest % 1_000)
}

#[cfg(test)]
mod tests {
    use super::{format_elapsed, timeval_to_duration};
    use std::time::Duration;

    /// The shape bash prints: minutes are never zero-padded, seconds always are.
    #[test]
    fn elapsed_is_formatted_as_minutes_and_millis() {
        assert_eq!(format_elapsed(Duration::ZERO), "0m0.000s");
        assert_eq!(format_elapsed(Duration::from_millis(7)), "0m0.007s");
        assert_eq!(format_elapsed(Duration::from_millis(1234)), "0m1.234s");
        assert_eq!(format_elapsed(Duration::from_secs(59)), "0m59.000s");
        assert_eq!(format_elapsed(Duration::from_secs(60)), "1m0.000s");
        assert_eq!(
            format_elapsed(Duration::from_millis(3_661_500)),
            "61m1.500s"
        );
    }

    /// Sub-millisecond time truncates to `0m0.000s`; it must not borrow into the minutes field.
    #[test]
    fn sub_millisecond_time_is_zero_not_negative() {
        assert_eq!(format_elapsed(Duration::from_micros(999)), "0m0.000s");
    }

    /// A negative `timeval` is nonsense the kernel should never produce, but `Duration::new` would
    /// panic on the cast, so it is clamped instead of trusted.
    #[test]
    fn negative_timeval_clamps_to_zero() {
        let tv = nix::libc::timeval {
            tv_sec: -5,
            tv_usec: -1,
        };
        assert_eq!(timeval_to_duration(tv), Duration::ZERO);
    }

    /// Microseconds are scaled, not copied: 1_500 µs is 1.5 ms, not 1.5 s.
    #[test]
    fn timeval_microseconds_are_scaled() {
        let tv = nix::libc::timeval {
            tv_sec: 2,
            tv_usec: 1_500,
        };
        assert_eq!(timeval_to_duration(tv), Duration::from_micros(2_001_500));
    }
}

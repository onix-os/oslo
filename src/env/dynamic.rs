//! Variables whose value is read at the moment they are expanded.
//!
//! `$EPOCHREALTIME` is not a variable anybody set; it is the clock, spelled as a variable. So it
//! cannot live in the variable table — it has to be computed on each expansion, which is what this
//! module is for.
//!
//! # Why a shell needs these at all
//!
//! Because every prompt tool measures how long your last command took, and the only portable way
//! to do that is to read a high-resolution clock twice: once in preexec, once in precmd. Without
//! `$EPOCHREALTIME`, starship shells out to `starship time`, oh-my-posh to `oh-my-posh get millis`
//! and hexe to `date +%s%3N` — **a process fork per command, on the path between pressing Enter and
//! seeing a prompt.** That is the whole cost of not having a two-line feature.
//!
//! `$SECONDS` is deliberately here too, and deliberately useless for that purpose: bash specifies
//! it at one-second resolution, so it measures nothing a person would notice. It exists because
//! scripts use it for coarse timeouts.
//!
//! # An assignment wins
//!
//! Every name here is overridable. `SECONDS=0` is an idiom — it resets the count — and a script
//! that says `RANDOM=42` is asking for a reproducible sequence. So the table is consulted only
//! when the variable has not been set, which also means none of these can shadow something a
//! parent process exported.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// When the shell started, for `$SECONDS`.
static STARTED: AtomicU64 = AtomicU64::new(0);

/// The seed `$RANDOM` walks, so a sequence is a sequence rather than the same number twice.
static SEED: AtomicU64 = AtomicU64::new(0);

/// Record the start time and seed the generator. Called once, from `Environment::new`.
pub fn start() {
    STARTED.store(now_secs(), Ordering::Relaxed);
    // Seeded from the clock and the pid, so two shells started in the same second still differ.
    let seed = now_nanos() ^ (u64::from(std::process::id()) << 32);
    SEED.store(seed | 1, Ordering::Relaxed);
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn now_nanos() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

/// The value of `name` if it is one of these, or `None`.
pub fn value(name: &str) -> Option<String> {
    Some(match name {
        // Six decimal places, and **always a `.`** — bash's is locale-dependent, which is why
        // oh-my-posh strips every non-digit before parsing it. A prompt tool reading this one can
        // just split on the dot.
        "EPOCHREALTIME" => {
            let d = SystemTime::now().duration_since(UNIX_EPOCH).ok()?;
            format!("{}.{:06}", d.as_secs(), d.subsec_micros())
        }
        "EPOCHSECONDS" => now_secs().to_string(),
        // Whole seconds since the shell started, as bash specifies it. Useless for timing a
        // command and fine for "have we been waiting five minutes".
        "SECONDS" => now_secs()
            .saturating_sub(STARTED.load(Ordering::Relaxed))
            .to_string(),
        // 0..32767, bash's range. xorshift64* rather than anything from a crate: this is a shell
        // variable for picking a temp-file suffix, not a source of randomness anybody should be
        // trusting, and saying so in the code is better than implying otherwise with a dependency.
        "RANDOM" => (next_random() % 32768).to_string(),
        // 32 bits from the OS, for the cases `$RANDOM` is too weak and too narrow for. bash reads
        // `getrandom`; falling back to the same generator is honest about what it is.
        "SRANDOM" => match getrandom_u32() {
            Some(n) => n.to_string(),
            None => (next_random() as u32).to_string(),
        },
        _ => return None,
    })
}

/// Whether `name` is one of ours, without computing it.
pub fn is_dynamic(name: &str) -> bool {
    matches!(
        name,
        "EPOCHREALTIME" | "EPOCHSECONDS" | "SECONDS" | "RANDOM" | "SRANDOM"
    )
}

/// xorshift64*, stepped once.
fn next_random() -> u64 {
    let mut x = SEED.load(Ordering::Relaxed);
    x ^= x >> 12;
    x ^= x << 25;
    x ^= x >> 27;
    SEED.store(x, Ordering::Relaxed);
    x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 33
}

/// Four bytes from the kernel, or `None` where that is not available.
fn getrandom_u32() -> Option<u32> {
    let mut bytes = [0u8; 4];
    // SAFETY: a four-byte buffer this call owns, and `getrandom` writes at most that many.
    let n = unsafe {
        nix::libc::getrandom(
            bytes.as_mut_ptr().cast(),
            bytes.len(),
            nix::libc::GRND_NONBLOCK,
        )
    };
    (n == bytes.len() as isize).then(|| u32::from_ne_bytes(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_clock_variables_are_the_clock() {
        start();
        let real = value("EPOCHREALTIME").expect("set");
        let (secs, micros) = real.split_once('.').expect("always a dot, never a comma");
        assert!(secs.parse::<u64>().expect("seconds") > 1_600_000_000);
        assert_eq!(micros.len(), 6, "six places, always: {real:?}");
        assert!(micros.chars().all(|c| c.is_ascii_digit()));

        let whole: u64 = value("EPOCHSECONDS").expect("set").parse().expect("number");
        assert_eq!(whole.to_string(), secs, "the two must agree");
    }

    /// It has to actually advance, or a duration computed from it is always zero.
    #[test]
    fn the_clock_moves() {
        start();
        let first = value("EPOCHREALTIME").expect("set");
        std::thread::sleep(std::time::Duration::from_millis(2));
        let second = value("EPOCHREALTIME").expect("set");
        assert_ne!(first, second, "a duration measured with this would be 0");
    }

    #[test]
    fn seconds_starts_at_zero() {
        start();
        assert_eq!(value("SECONDS").as_deref(), Some("0"));
    }

    /// A sequence, not the same number repeatedly, and inside bash's range.
    #[test]
    fn random_is_a_sequence_in_bashs_range() {
        start();
        let draws: Vec<u64> = (0..64)
            .map(|_| value("RANDOM").expect("set").parse().expect("number"))
            .collect();
        assert!(draws.iter().all(|&n| n < 32768), "outside 0..32767");
        assert!(
            draws.windows(2).any(|w| w[0] != w[1]),
            "the same number every time: {draws:?}"
        );
    }

    #[test]
    fn srandom_is_wider() {
        start();
        let n: u64 = value("SRANDOM").expect("set").parse().expect("number");
        assert!(n <= u64::from(u32::MAX));
    }

    #[test]
    fn nothing_else_is_dynamic() {
        assert!(is_dynamic("EPOCHREALTIME"));
        assert!(is_dynamic("RANDOM"));
        assert!(!is_dynamic("PATH"));
        assert!(!is_dynamic("epochrealtime"), "names are case-sensitive");
        assert_eq!(value("PATH"), None);
    }
}

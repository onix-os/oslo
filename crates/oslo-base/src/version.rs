//! The shell's version, as one number.
//!
//! # Why this needs a module at all
//!
//! `env!("CARGO_PKG_VERSION")` is the version of *the crate it is written in*. oslo is a workspace,
//! so there are several — and they disagree: the binary was 0.2.29 while `oslo.version`, read from
//! `oslo-runtime`, answered 0.2.21. A shell that reports two versions is one where nobody can say
//! which release they are running, and a plugin declaring `requires = ">= 0.2.29"` would be checked
//! against a number its author never saw.
//!
//! So there is one: the **binary's**, installed by `main` at startup and read from everywhere else.

use std::sync::OnceLock;

static VERSION: OnceLock<&'static str> = OnceLock::new();

/// Declare the version this binary was built as. Called once, by `main`.
///
/// Later calls are ignored, so nothing can change the shell's version while it runs.
pub fn install(version: &'static str) {
    let _ = VERSION.set(version);
}

/// The shell's version.
///
/// Falls back to this crate's own when `main` never ran — a unit test, or a library user. That is a
/// number that exists rather than a guess, and the fallback cannot be reached by the binary.
pub fn current() -> &'static str {
    VERSION.get().copied().unwrap_or(env!("CARGO_PKG_VERSION"))
}

/// A version as the three numbers that order it.
///
/// Anything after a `-` or `+` is ignored: `0.3.0-rc1` compares as `0.3.0`, because a requirement
/// is about the interface and a pre-release of 0.3.0 has 0.3.0's interface. Missing parts are zero,
/// so `0.3` is `0.3.0`.
pub fn numbers(version: &str) -> Option<(u64, u64, u64)> {
    let core = version.split(['-', '+']).next().unwrap_or(version).trim();
    let mut parts = core.split('.');
    let mut next = || -> Option<u64> {
        match parts.next() {
            Some(part) => part.trim().parse().ok(),
            None => Some(0),
        }
    };
    let (major, minor, patch) = (next()?, next()?, next()?);
    // A fourth part is not a version this understands, and guessing which three of four were meant
    // is worse than saying so.
    parts.next().is_none().then_some((major, minor, patch))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_version_orders_by_its_three_numbers() {
        assert_eq!(numbers("0.2.29"), Some((0, 2, 29)));
        assert!(numbers("0.2.29") < numbers("0.3.0"));
        assert!(numbers("0.2.9") < numbers("0.2.29"), "not string order");
        assert!(numbers("1.0.0") > numbers("0.99.99"));
    }

    #[test]
    fn a_missing_part_is_zero_and_a_pre_release_is_its_own_version() {
        assert_eq!(numbers("0.3"), Some((0, 3, 0)));
        assert_eq!(numbers("1"), Some((1, 0, 0)));
        assert_eq!(numbers("0.3.0-rc1"), Some((0, 3, 0)));
        assert_eq!(numbers("0.3.0+build7"), Some((0, 3, 0)));
    }

    #[test]
    fn something_that_is_not_a_version_is_refused_rather_than_guessed_at() {
        for bad in ["", "x", "0.x.1", "1.2.3.4", "0..1"] {
            assert_eq!(numbers(bad), None, "{bad:?} was read as a version");
        }
    }

    #[test]
    fn the_current_version_is_a_version() {
        assert!(numbers(current()).is_some(), "{}", current());
    }
}

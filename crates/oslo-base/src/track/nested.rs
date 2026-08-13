//! How many oslo shells deep this one is, published as `$OSLO_NESTED`.
//!
//! ```sh
//! oslo            # $OSLO_NESTED is 0
//!   oslo          # 1, and it asks first — see `startup::nested`
//!     oslo        # 2
//! ```
//!
//! # Why a shell needs to be told
//!
//! Nothing on the screen says which shell you are typing at. Exiting one leaves you at a prompt
//! that looks identical to the one you were at a moment ago, so a shell nested by accident is
//! discovered later, usually by an `exit` that did not close the terminal. The count exists so a
//! prompt can say so: `[ "$OSLO_NESTED" -gt 0 ] && …`.
//!
//! # Zero is exported too
//!
//! The outermost shell publishes `0` rather than leaving the variable unset, because the variable
//! is also how the next oslo knows there *is* an outer one. Absent means "no oslo above this",
//! which is exactly what a fresh terminal should say.
//!
//! Every shell publishes it, `-c` and scripts included — it counts the oslo shells above this
//! process, and one started by a script is one of them. Only an interactive shell asks about it.

use std::sync::OnceLock;

/// The variable, named once so nothing spells it a second way.
pub const VARIABLE: &str = "OSLO_NESTED";

/// This shell's own depth, fixed for its lifetime.
static DEPTH: OnceLock<usize> = OnceLock::new();

/// Take a place in the stack and export it, for a process that *is* a shell.
///
/// Beside [`super::session::begin`] and called from the same places, for the same reason: a tool
/// oslo starts is not a shell and must not count as one, or `oslo macros` would tell every shell it
/// opened that it was a level deeper than it is.
pub fn begin() -> usize {
    let level = next(inherited());
    // SAFETY: called once, from `main`, before any thread is started — as `session::begin` is.
    unsafe { std::env::set_var(VARIABLE, level.to_string()) };
    *DEPTH.get_or_init(|| level)
}

/// How deep this shell is: `0` at the top, `1` inside one oslo, and so on.
pub fn depth() -> usize {
    *DEPTH.get_or_init(|| inherited().unwrap_or(0))
}

/// The depth of the shell that started this process, if one did.
fn inherited() -> Option<usize> {
    std::env::var(VARIABLE).ok()?.trim().parse().ok()
}

/// The level below `above`.
///
/// A value that is not a number is treated as no value at all: something other than oslo wrote it,
/// and guessing what it meant would put a shell at a depth nobody can explain.
fn next(above: Option<usize>) -> usize {
    above.map_or(0, |level| level.saturating_add(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The arithmetic alone, which is all of it that can be checked without changing the
    /// environment of every other test's thread.
    #[test]
    fn the_first_shell_is_the_zeroth() {
        assert_eq!(next(None), 0, "a fresh terminal has nothing above it");
        assert_eq!(next(Some(0)), 1);
        assert_eq!(next(Some(7)), 8);
    }

    /// Somebody else's `OSLO_NESTED=deep` is not a depth, and this shell is not nested in it.
    #[test]
    fn a_value_that_is_not_a_number_is_not_a_shell() {
        assert_eq!(next(None), 0);
        assert_eq!(next(usize::MAX.into()), usize::MAX, "no wrap to zero");
    }
}

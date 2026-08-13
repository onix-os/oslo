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
//! # One terminal, one stack
//!
//! Inheritance alone says a `tmux` pane, a `hexe` pod and an `ssh` login are all nested — the
//! variable travels into every one of them, because each is started by a shell that had it. None of
//! them is: they are looking at a screen of their own, and the shell that opened them is not
//! something they can `exit` back into.
//!
//! So the count travels with the terminal it was set on — `$OSLO_NESTED_TTY`, the controlling
//! terminal's device number — and a count that arrives from a different one starts again at the top.
//!
//! Every shell publishes it, `-c` and scripts included — it counts the oslo shells above this
//! process, and one started by a script is one of them. Only an interactive shell asks about it.

use std::sync::OnceLock;

/// The variable, named once so nothing spells it a second way.
pub const VARIABLE: &str = "OSLO_NESTED";

/// Which terminal the shell that published [`VARIABLE`] was looking at.
///
/// **The environment alone cannot answer this.** A variable is inherited by everything a shell
/// starts, and `tmux`, `hexe` and `ssh` are all things a shell starts — so a new pane and a remote
/// login arrive carrying a count from a shell they are not inside. They were being asked whether
/// they meant to nest, and they would have gone on showing `⧉1` in a prompt for ever.
///
/// What they do not carry is the *screen*: every one of them runs its shell on a pty of its own. So
/// the count travels with the controlling terminal it was set on, and a count from another one is
/// somebody else's.
const TERMINAL: &str = "OSLO_NESTED_TTY";

/// This shell's own depth, fixed for its lifetime.
static DEPTH: OnceLock<usize> = OnceLock::new();

/// Take a place in the stack and export it, for a process that *is* a shell.
///
/// Beside [`super::session::begin`] and called from the same places, for the same reason: a tool
/// oslo starts is not a shell and must not count as one, or `oslo macros` would tell every shell it
/// opened that it was a level deeper than it is.
pub fn begin() -> usize {
    let here = terminal();
    let level = next(inherited(), here_too(here.as_deref()));
    // SAFETY: called once, from `main`, before any thread is started — as `session::begin` is.
    unsafe {
        std::env::set_var(VARIABLE, level.to_string());
        match &here {
            Some(tty) => std::env::set_var(TERMINAL, tty),
            // Removed rather than left behind: an inherited terminal that is not ours would make
            // the next shell in this chain believe it shares a screen with something it cannot see.
            None => std::env::remove_var(TERMINAL),
        }
    }
    *DEPTH.get_or_init(|| level)
}

/// How deep this shell is: `0` at the top, `1` inside one oslo, and so on.
///
/// Asked before [`begin`] — by a tool rather than a shell — it answers with the depth of the shell
/// this process belongs to, which is the inherited one and not a level below it.
pub fn depth() -> usize {
    *DEPTH.get_or_init(|| {
        inherited()
            .filter(|_| here_too(terminal().as_deref()))
            .unwrap_or(0)
    })
}

/// The depth of the shell that started this process, if one did.
fn inherited() -> Option<usize> {
    std::env::var(VARIABLE).ok()?.trim().parse().ok()
}

/// The controlling terminal, as the number the kernel knows it by.
///
/// `/dev/tty` is whichever terminal *this process* is attached to, whatever its own descriptors
/// have been redirected to, so this survives a pipeline and answers `None` only where there is
/// genuinely no terminal — a service, a cron job, a CI runner.
fn terminal() -> Option<String> {
    use std::os::unix::fs::MetadataExt;
    let tty = std::fs::File::open("/dev/tty").ok()?;
    Some(tty.metadata().ok()?.rdev().to_string())
}

/// Whether the shell that published the count was looking at the same screen this one is.
///
/// **Neither having a terminal counts as the same**, which is what keeps a chain of scripts
/// counting: two shells in a CI runner are as nested as two shells in a terminal, and there is
/// nobody there for the difference to matter to. One having one and the other not is a change of
/// screen like any other.
fn here_too(here: Option<&str>) -> bool {
    std::env::var(TERMINAL).ok().as_deref() == here
}

/// The level below `above`, or the top when this is a screen of its own.
///
/// A value that is not a number is treated as no value at all: something other than oslo wrote it,
/// and guessing what it meant would put a shell at a depth nobody can explain.
fn next(above: Option<usize>, same_terminal: bool) -> usize {
    match above {
        Some(level) if same_terminal => level.saturating_add(1),
        // A new terminal is a new stack: a tmux pane, a hexe pod and an ssh login all begin at the
        // top however deep the shell that opened them was.
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The arithmetic alone, which is all of it that can be checked without changing the
    /// environment of every other test's thread.
    #[test]
    fn the_first_shell_is_the_zeroth() {
        assert_eq!(next(None, true), 0, "a fresh terminal has nothing above it");
        assert_eq!(next(Some(0), true), 1);
        assert_eq!(next(Some(7), true), 8);
    }

    /// **A new terminal is a new stack.** A tmux pane, a hexe pod and an ssh login all inherit the
    /// count from the shell that opened them and are not inside it — they are looking at a screen
    /// of their own, and asking them whether they meant to nest is asking about somebody else.
    #[test]
    fn another_screen_starts_again_at_the_top() {
        assert_eq!(next(Some(0), false), 0);
        assert_eq!(
            next(Some(3), false),
            0,
            "however deep the one that opened it"
        );
    }

    /// Somebody else's `OSLO_NESTED=deep` is not a depth, and this shell is not nested in it.
    #[test]
    fn a_value_that_is_not_a_number_is_not_a_shell() {
        assert_eq!(next(None, true), 0);
        assert_eq!(next(Some(usize::MAX), true), usize::MAX, "no wrap to zero");
    }
}

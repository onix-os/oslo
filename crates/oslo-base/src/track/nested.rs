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
//! So the count travels with the terminal it was set on — `$OSLO_NESTED_TTY`, this host and the
//! device this terminal *is* — and a count that arrives from a different one starts again at the top.
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
/// the count travels with the terminal it was set on, and a count from another one is somebody
/// else's. What names a terminal here is [`terminal`], and getting that wrong made this check pass
/// for every pane on the machine.
const TERMINAL: &str = "OSLO_NESTED_TTY";

/// The shell that published the count, by process id.
///
/// **The terminal alone is not enough, because an environment can outlive the thing it describes.**
/// A `tmux` or `hexe` server keeps the variables it was started with for as long as it runs, and
/// hands them to every pane it opens for the rest of the week — so a count can arrive from a shell
/// that exited days ago. A pid can be checked: the shell you are supposedly inside has to be a
/// process that is still running *and* still between this one and the terminal, or there is nothing
/// to `exit` back into.
const OWNER: &str = "OSLO_NESTED_PID";

/// This shell's own depth, fixed for its lifetime.
static DEPTH: OnceLock<usize> = OnceLock::new();

/// Take a place in the stack and export it, for a process that *is* a shell.
///
/// Beside [`super::session::begin`] and called from the same places, for the same reason: a tool
/// oslo starts is not a shell and must not count as one, or `oslo macros` would tell every shell it
/// opened that it was a level deeper than it is.
pub fn begin() -> usize {
    let here = terminal();
    let level = next(inherited(), inside(here.as_deref()));
    // SAFETY: called once, from `main`, before any thread is started — as `session::begin` is.
    unsafe {
        std::env::set_var(VARIABLE, level.to_string());
        std::env::set_var(OWNER, std::process::id().to_string());
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
            .filter(|_| inside(terminal().as_deref()))
            .unwrap_or(0)
    })
}

/// The depth of the shell that started this process, if one did.
fn inherited() -> Option<usize> {
    std::env::var(VARIABLE).ok()?.trim().parse().ok()
}

/// This terminal, named by the device it actually is: `tron:88:6` for `/dev/pts/6`.
///
/// **Not `/dev/tty`.** That was the first attempt and it is wrong in the worst way — it names a
/// terminal that exists on every machine and answers `5:0` from *all* of them, so every pane, every
/// login and every new window matched every other one and the whole check passed for reasons that
/// had nothing to do with the screen. Measured, after the reports came in:
///
/// ```text
/// /dev/pts/6   stat /dev/tty            → 5:0     ← the same everywhere
///              stat /proc/self/fd/0     → 88:6    ← this pty and no other
/// ```
///
/// The host is in front of it because a device number is only unique on the machine that issued it:
/// `/dev/pts/6` here and `/dev/pts/6` on the far end of an `ssh` are both `88:6`, and a variable
/// that ever reaches the other machine must not make one look like the other.
///
/// `None` where there is no terminal at all — a service, a cron job, a CI runner.
fn terminal() -> Option<String> {
    use std::os::unix::fs::MetadataExt;
    // stdin, then stderr: the two the question itself needs, so a shell that has one to draw on
    // always has a name for it. Anything else — a pipe, a file, `/dev/null` — is not a terminal
    // and must not be mistaken for one that two unrelated shells happen to share.
    [0, 2].into_iter().find_map(|fd| {
        if !nix::unistd::isatty(fd).unwrap_or(false) {
            return None;
        }
        let device = std::fs::metadata(format!("/proc/self/fd/{fd}"))
            .ok()?
            .rdev();
        Some(format!("{}:{device}", super::session::host()))
    })
}

/// Whether this shell really is inside the one that published the count.
///
/// Two things have to hold, and each catches what the other cannot:
///
/// * **The same screen.** Neither having a terminal counts as the same, which is what keeps a chain
///   of scripts counting: two shells in a CI runner are as nested as two in a terminal, and there is
///   nobody there for the difference to matter to.
/// * **A live ancestor.** The publisher has to still be running and still be between this process
///   and the terminal. An environment outlives what it describes — a `tmux` server hands out the
///   variables it started with for as long as it runs — and this is the half that a stale one
///   cannot survive.
fn inside(here: Option<&str>) -> bool {
    if std::env::var(TERMINAL).ok().as_deref() != here {
        return false;
    }
    match std::env::var(OWNER)
        .ok()
        .and_then(|pid| pid.trim().parse().ok())
    {
        Some(pid) => ancestor(pid),
        // No owner named at all: an older oslo published the count, or something else did. The
        // terminal has already agreed, and refusing here would make the count useless during an
        // upgrade — the shell above is on this screen, which is what was asked.
        None => true,
    }
}

/// Whether `pid` is a process this one descends from.
///
/// Walked through `/proc`, one parent at a time, which is the only place the answer exists. Bounded
/// because a corrupt chain must not become a loop; nothing real is sixty-four processes deep.
fn ancestor(pid: u32) -> bool {
    let mut current = std::process::id();
    for _ in 0..64 {
        let Some(parent) = parent_of(current) else {
            return false;
        };
        if parent == pid {
            return true;
        }
        if parent <= 1 {
            return false;
        }
        current = parent;
    }
    false
}

/// The parent of `pid`, from `/proc/<pid>/stat`.
///
/// Read after the last `)`, because the second field is the executable's name and a name is allowed
/// to contain both spaces and parentheses — splitting from the left is the classic way to read the
/// wrong number out of this file.
fn parent_of(pid: u32) -> Option<u32> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let after_name = stat.rsplit_once(')')?.1;
    // state, then ppid.
    after_name.split_whitespace().nth(1)?.parse().ok()
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

    /// **The shell above has to still be there.** A `tmux` server hands out the environment it was
    /// started with for as long as it runs, so a count can arrive from a shell that exited days
    /// ago; a pid is the half of the check that cannot be stale.
    #[test]
    fn only_a_live_ancestor_counts() {
        assert!(
            ancestor(std::os::unix::process::parent_id()),
            "the process that started this test is an ancestor of it"
        );
        assert!(
            !ancestor(std::process::id()),
            "a process is not inside itself"
        );
        // A pid that cannot be running: nothing is above the maximum, and a chain that ends at
        // `init` ends rather than looping.
        assert!(!ancestor(u32::MAX));
    }

    /// The name in `/proc/<pid>/stat` may contain spaces and parentheses, which is why the fields
    /// are read from the right of it.
    #[test]
    fn the_parent_is_read_past_the_name() {
        let mine = parent_of(std::process::id()).expect("this process has a parent");
        assert_eq!(mine, std::os::unix::process::parent_id());
    }
}

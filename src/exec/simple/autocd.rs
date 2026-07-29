//! `shopt -s autocd`: a command word that names a directory means `cd` to it.
//!
//! Its own module because it is a self-contained *option* with three separate gates — the switch,
//! the shell being interactive, and the word being a bare name — and because the switch has to
//! live somewhere `shopt` can reach without going through the `set -o` table, which carries only
//! the POSIX option set (bash does not put autocd there either).
//!
//! Off by default, for the reason bash has it off: in a script `build` means "run the build
//! command", and silently changing directory instead makes every later relative path resolve
//! somewhere else, with status 0 to say all is well (PLAN R5.13).

use crate::env::Environment;
use crate::error::Result;
use std::sync::atomic::{AtomicBool, Ordering};

/// The switch itself.
///
/// Process-global rather than a field on [`Environment`], like
/// [`crate::exec::pipeline::set_interactive`]: it is a property of the invocation, and a forked
/// child inherits it as it stands.
static AUTOCD: AtomicBool = AtomicBool::new(false);

/// Enable or disable autocd. This is what `shopt -s autocd` and `shopt -u autocd` call — the
/// hook had no callers at all until `shopt` existed, which meant autocd could never be switched
/// on by any spelling bash understands (PLAN C7).
///
/// Switching it on is not enough on its own: the shell must also be interactive, and the command
/// word must be a bare name with no arguments.
pub fn set_autocd(yes: bool) {
    AUTOCD.store(yes, Ordering::Relaxed);
}

/// Whether autocd has been switched on.
///
/// Private: `shopt -p autocd` answers from `shopt`'s own bitset, and a second public reader here
/// would be a second thing to keep in step for no caller's benefit.
fn autocd() -> bool {
    AUTOCD.load(Ordering::Relaxed)
}

/// `cd` to `cmd_name` if this shell is allowed to guess that is what was meant.
///
/// `None` — the overwhelmingly common answer — leaves the caller to report the real failure.
/// Three conditions, all required: the shell is interactive (a person is there to see the
/// directory change and to have meant it), autocd is switched on, and the command is a bare
/// word with no arguments, since `build --release` is unambiguously not a `cd`.
pub(super) fn try_autocd(
    env: &mut Environment,
    cmd_name: &str,
    words: &[String],
) -> Option<Result<i32>> {
    if words.len() != 1 || !enabled(env) {
        return None;
    }
    if !std::path::Path::new(cmd_name).is_dir() {
        return None;
    }
    Some(crate::env::builtins::builtin_cd(
        env,
        &["cd".to_string(), cmd_name.to_string()],
    ))
}

/// Whether autocd may fire: interactive *and* opted in.
///
/// The interactive half is not configurable, in bash either — `bash -O autocd -c 'somedir'`
/// still reports `command not found`. A script's meaning must not depend on which directories
/// happen to exist beside it.
///
/// `RUSH_AUTOCD` is the second way in, and it predates `shopt`: either switch is enough.
fn enabled(env: &Environment) -> bool {
    if !crate::exec::pipeline::is_interactive() {
        return false;
    }
    autocd()
        || env
            .get_var("RUSH_AUTOCD")
            .is_some_and(|v| !v.is_empty() && v != "0")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The switch round-trips, and it is *not* the whole answer: a non-interactive shell ignores
    /// it, which is what stops a script's meaning depending on the directories beside it. The
    /// unit-test process is not an interactive shell, so `enabled` stays false either way.
    #[test]
    fn the_switch_round_trips_but_does_not_reach_a_script() {
        let env = Environment::new();
        assert!(!autocd());
        assert!(!enabled(&env));

        set_autocd(true);
        assert!(autocd());
        assert!(!enabled(&env), "a non-interactive shell never autocds");

        set_autocd(false);
        assert!(!autocd());
    }
}

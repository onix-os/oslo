//! How the rest of the shell asks about `set -e`, `set -u`, `set -o pipefail` and friends.
//!
//! An `impl` block of its own file: the accessors are the API three separate rounds of work hang
//! off (errexit in the evaluator, nounset in expansion, noclobber in redirection, pipefail in the
//! pipeline), and they are cheap enough — a bit test on a `Copy` bitset — that no caller needs to
//! cache the answer.
//!
//! # Which accessor to use
//!
//! * the named ones ([`Environment::errexit`], [`Environment::nounset`], …) for the options that
//!   have behaviour attached, because a reader of the call site should not have to know what
//!   `ShellOption::NoClobber` is;
//! * [`Environment::option`] for anything else, including options rush only stores.
//!
//! Nothing here *acts* on an option. Storage and behaviour are deliberately separate: `set -x`
//! must be accepted, reported by `$-` and listed by `set -o` whether or not the tracing code
//! exists yet.

use super::Environment;
use crate::env::options::{ShellOption, ShellOptions};

impl Environment {
    /// Every option currently in force. `Copy`, so this does not borrow the environment.
    pub fn options(&self) -> ShellOptions {
        self.options
    }

    /// Replace the whole option set, e.g. when restoring it after a subshell-like construct.
    pub fn set_options(&mut self, options: ShellOptions) {
        self.options = options;
    }

    /// Whether `option` is on.
    pub fn option(&self, option: ShellOption) -> bool {
        self.options.is_set(option)
    }

    /// Turn `option` on or off. The only writer besides [`Environment::set_options`].
    pub fn set_option(&mut self, option: ShellOption, on: bool) {
        self.options.set(option, on);
    }

    /// `set -e`: a command that fails ends the shell.
    pub fn errexit(&self) -> bool {
        self.option(ShellOption::ErrExit)
    }

    /// `set -u`: expanding an unset parameter is an error.
    pub fn nounset(&self) -> bool {
        self.option(ShellOption::NoUnset)
    }

    /// `set -x`: print each expanded command to stderr, prefixed with `$PS4`.
    pub fn xtrace(&self) -> bool {
        self.option(ShellOption::XTrace)
    }

    /// `set -v`: echo each line of input as it is read.
    pub fn verbose(&self) -> bool {
        self.option(ShellOption::Verbose)
    }

    /// `set -n`: read and parse commands but do not run them.
    pub fn noexec(&self) -> bool {
        self.option(ShellOption::NoExec)
    }

    /// `set -f`: do not expand pathnames.
    pub fn noglob(&self) -> bool {
        self.option(ShellOption::NoGlob)
    }

    /// `set -C`: `>` refuses to truncate an existing file; only `>|` may.
    pub fn noclobber(&self) -> bool {
        self.option(ShellOption::NoClobber)
    }

    /// `set -o pipefail`: a pipeline reports the rightmost non-zero stage, not the last stage.
    pub fn pipefail(&self) -> bool {
        self.option(ShellOption::PipeFail)
    }

    /// `set -a`: every assignment is exported.
    pub fn allexport(&self) -> bool {
        self.option(ShellOption::AllExport)
    }

    /// `set -m`: run jobs in their own process groups with job-control notification.
    pub fn monitor(&self) -> bool {
        self.option(ShellOption::Monitor)
    }

    /// Whether this shell is interactive. Set from the invocation; `set` cannot change it.
    pub fn interactive(&self) -> bool {
        self.option(ShellOption::Interactive)
    }

    /// The value of `$-`.
    pub fn option_flags(&self) -> String {
        self.options.flag_string()
    }
}

#[cfg(test)]
mod tests {
    use crate::env::Environment;
    use crate::env::options::ShellOption;

    #[test]
    fn options_start_off_and_survive_a_round_trip() {
        let mut env = Environment::new();
        assert!(!env.errexit() && !env.nounset() && !env.pipefail());
        assert_eq!(env.option_flags(), "");

        env.set_option(ShellOption::ErrExit, true);
        assert!(env.errexit());
        assert_eq!(env.option_flags(), "e");

        let saved = env.options();
        env.set_option(ShellOption::ErrExit, false);
        assert!(!env.errexit());
        env.set_options(saved);
        assert!(env.errexit());
    }

    /// A subshell inherits the options: `set -e; (false)` has to see errexit in the child.
    #[test]
    fn entering_a_subshell_keeps_the_options() {
        let mut env = Environment::new();
        env.set_option(ShellOption::NoUnset, true);
        env.enter_subshell();
        assert!(env.nounset());
    }
}

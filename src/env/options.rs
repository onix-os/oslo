//! Shell options: the set behind `set -e`, `set -o pipefail` and `$-`.
//!
//! One table ([`ALL`]) is the single source of truth for every way an option can be named — the
//! single letter `set -e` uses, the long name `set -o errexit` uses, and the character `$-`
//! reports. A second list anywhere else is how a shell ends up accepting `set -o errexit` while
//! `$-` still says the option is off.
//!
//! # Querying an option
//!
//! Storage is a bitset, so a query is a bit test and callers can take a copy. The intended entry
//! points are the accessors on [`Environment`], not this type:
//!
//! ```
//! # use rush::env::Environment;
//! # use rush::env::options::ShellOption;
//! # let mut env = Environment::new();
//! env.set_option(ShellOption::ErrExit, true);
//! assert!(env.errexit()); // the named accessor, for the options with behaviour
//! assert!(env.option(ShellOption::ErrExit)); // the general form, for the rest
//! ```
//!
//! [`Environment`]: crate::env::scope::Environment

mod apply;

pub use apply::{SetArgs, SetError, SetListing, parse_set_args};

/// Every option this shell knows, whether or not it does anything yet.
///
/// Listing an option here is a promise about *naming*, not about behaviour: `set -o pipefail`
/// must be accepted and reported by `$-`/`set -o` even in the release where the pipeline code
/// has not been taught to read it, because a script that turns an option on and gets "invalid
/// option" back is broken in a way that a script whose option is a no-op is not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellOption {
    AllExport,
    Notify,
    ErrExit,
    NoGlob,
    HashAll,
    Keyword,
    Monitor,
    NoExec,
    OneCmd,
    NoUnset,
    Verbose,
    XTrace,
    NoClobber,
    IgnoreEof,
    NoLog,
    PipeFail,
    Posix,
    Vi,
    Emacs,
    /// The shell is talking to a person. Set from the invocation, never by `set`.
    Interactive,
    /// The program came from `-c`. Set from the invocation, never by `set`.
    CommandString,
    /// The program is being read from standard input. Set from the invocation, never by `set`.
    StdinInput,
}

/// How an option can be written.
pub struct OptionSpec {
    pub option: ShellOption,
    /// The `set -x` letter, and the character `$-` reports. Some options have only a long name.
    pub letter: Option<char>,
    /// The `set -o` name. `None` for the three invocation flags, which `set` cannot change.
    pub name: Option<&'static str>,
}

impl OptionSpec {
    /// Whether `set` may change this option. False only for the invocation flags.
    pub fn settable(&self) -> bool {
        self.name.is_some()
    }
}

const fn spec(option: ShellOption, letter: Option<char>, name: Option<&'static str>) -> OptionSpec {
    OptionSpec {
        option,
        letter,
        name,
    }
}

/// Every option, in the order `$-` and `set -o` report them.
///
/// Sorted by letter with the letterless options after the ones that have letters, which is what
/// bash's own ordering amounts to; the three invocation flags come last because `$-` puts them
/// there (`bash -c 'echo $-'` prints `hBc`, not `chB`).
pub const ALL: &[OptionSpec] = &[
    spec(ShellOption::AllExport, Some('a'), Some("allexport")),
    spec(ShellOption::Notify, Some('b'), Some("notify")),
    spec(ShellOption::ErrExit, Some('e'), Some("errexit")),
    spec(ShellOption::NoGlob, Some('f'), Some("noglob")),
    spec(ShellOption::HashAll, Some('h'), Some("hashall")),
    spec(ShellOption::Keyword, Some('k'), Some("keyword")),
    spec(ShellOption::Monitor, Some('m'), Some("monitor")),
    spec(ShellOption::NoExec, Some('n'), Some("noexec")),
    spec(ShellOption::OneCmd, Some('t'), Some("onecmd")),
    spec(ShellOption::NoUnset, Some('u'), Some("nounset")),
    spec(ShellOption::Verbose, Some('v'), Some("verbose")),
    spec(ShellOption::XTrace, Some('x'), Some("xtrace")),
    spec(ShellOption::NoClobber, Some('C'), Some("noclobber")),
    spec(ShellOption::IgnoreEof, None, Some("ignoreeof")),
    spec(ShellOption::NoLog, None, Some("nolog")),
    spec(ShellOption::PipeFail, None, Some("pipefail")),
    spec(ShellOption::Posix, None, Some("posix")),
    spec(ShellOption::Vi, None, Some("vi")),
    spec(ShellOption::Emacs, None, Some("emacs")),
    spec(ShellOption::Interactive, Some('i'), None),
    spec(ShellOption::CommandString, Some('c'), None),
    spec(ShellOption::StdinInput, Some('s'), None),
];

impl ShellOption {
    /// The option `set -LETTER` names, if any. Rejects the invocation flags: `set -i` is an
    /// error in bash, and accepting it would let a script claim the shell is interactive.
    pub fn from_letter(letter: char) -> Option<Self> {
        ALL.iter()
            .find(|s| s.settable() && s.letter == Some(letter))
            .map(|s| s.option)
    }

    /// The option `set -o NAME` names, if any.
    pub fn from_name(name: &str) -> Option<Self> {
        ALL.iter().find(|s| s.name == Some(name)).map(|s| s.option)
    }

    /// This option's `set -o` name, or `None` for the invocation flags.
    pub fn name(self) -> Option<&'static str> {
        self.spec().name
    }

    /// This option's letter, or `None` for the options only `set -o` can reach.
    pub fn letter(self) -> Option<char> {
        self.spec().letter
    }

    fn spec(self) -> &'static OptionSpec {
        ALL.iter()
            .find(|s| s.option == self)
            .expect("every ShellOption variant is in ALL")
    }

    /// Position in [`ALL`], which is also this option's bit in [`ShellOptions`].
    fn bit(self) -> u32 {
        ALL.iter()
            .position(|s| s.option == self)
            .expect("every ShellOption variant is in ALL") as u32
    }
}

/// The options currently in force, one bit each.
///
/// `Copy`, deliberately: a caller deep in expansion or redirection wants to ask a question about
/// the options without holding a borrow on the whole [`Environment`].
///
/// [`Environment`]: crate::env::scope::Environment
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ShellOptions {
    bits: u32,
}

impl ShellOptions {
    pub fn is_set(self, option: ShellOption) -> bool {
        self.bits & (1 << option.bit()) != 0
    }

    pub fn set(&mut self, option: ShellOption, on: bool) {
        if on {
            self.bits |= 1 << option.bit();
        } else {
            self.bits &= !(1 << option.bit());
        }
    }

    /// The value of `$-`: the letter of every set option that has one, in [`ALL`] order.
    ///
    /// Options with no letter (`pipefail`, `posix`, …) are invisible here — that is what `$-`
    /// means, and it is why `set -o` exists.
    pub fn flag_string(self) -> String {
        ALL.iter()
            .filter(|s| self.is_set(s.option))
            .filter_map(|s| s.letter)
            .collect()
    }

    /// `set -o` with no name: one line per settable option, `name` then a tab then `on`/`off`.
    ///
    /// Padded to bash's column width so the two shells' output can be compared directly.
    pub fn long_listing(self) -> String {
        self.listing(|name, on| format!("{:<15}\t{}\n", name, if on { "on" } else { "off" }))
    }

    /// `set +o`: the same states written as the commands that would restore them.
    ///
    /// This is the form POSIX requires to be re-inputtable, so `save=$(set +o)` … `eval "$save"`
    /// round-trips.
    pub fn reinputtable_listing(self) -> String {
        self.listing(|name, on| format!("set {}o {}\n", if on { '-' } else { '+' }, name))
    }

    fn listing(self, line: impl Fn(&str, bool) -> String) -> String {
        ALL.iter()
            .filter_map(|s| s.name.map(|name| (name, self.is_set(s.option))))
            .map(|(name, on)| line(name, on))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::{ALL, ShellOption, ShellOptions};

    /// The bitset indexes [`ALL`], so a duplicate entry would silently alias two options.
    #[test]
    fn every_option_appears_exactly_once() {
        for (i, spec) in ALL.iter().enumerate() {
            assert_eq!(
                ALL.iter().position(|s| s.option == spec.option),
                Some(i),
                "{:?} is listed twice",
                spec.option
            );
        }
        assert!(ALL.len() <= 32, "the bitset is a u32");
    }

    #[test]
    fn letters_and_names_are_unique() {
        for (i, spec) in ALL.iter().enumerate() {
            for other in &ALL[i + 1..] {
                assert!(
                    spec.letter.is_none() || spec.letter != other.letter,
                    "{:?} shares a letter with {:?}",
                    spec.option,
                    other.option
                );
                assert!(
                    spec.name.is_none() || spec.name != other.name,
                    "{:?} shares a name with {:?}",
                    spec.option,
                    other.option
                );
            }
        }
    }

    #[test]
    fn setting_one_option_leaves_the_others_alone() {
        let mut opts = ShellOptions::default();
        opts.set(ShellOption::ErrExit, true);
        opts.set(ShellOption::PipeFail, true);
        assert!(opts.is_set(ShellOption::ErrExit) && opts.is_set(ShellOption::PipeFail));
        assert!(!opts.is_set(ShellOption::NoUnset));
        opts.set(ShellOption::ErrExit, false);
        assert!(!opts.is_set(ShellOption::ErrExit));
        assert!(opts.is_set(ShellOption::PipeFail));
    }

    /// `set -i` must not be a way for a script to claim the shell is interactive.
    #[test]
    fn invocation_flags_have_no_set_spelling() {
        assert_eq!(ShellOption::from_letter('i'), None);
        assert_eq!(ShellOption::from_letter('c'), None);
        assert_eq!(ShellOption::from_name("interactive"), None);
        assert_eq!(ShellOption::from_letter('e'), Some(ShellOption::ErrExit));
        assert_eq!(
            ShellOption::from_name("pipefail"),
            Some(ShellOption::PipeFail)
        );
    }

    #[test]
    fn flag_string_is_letters_only_in_table_order() {
        let mut opts = ShellOptions::default();
        opts.set(ShellOption::XTrace, true);
        opts.set(ShellOption::ErrExit, true);
        opts.set(ShellOption::NoClobber, true);
        // pipefail has no letter, so it cannot show up in `$-`.
        opts.set(ShellOption::PipeFail, true);
        opts.set(ShellOption::CommandString, true);
        assert_eq!(opts.flag_string(), "exCc");
    }

    #[test]
    fn reinputtable_listing_round_trips_through_the_parser() {
        let mut opts = ShellOptions::default();
        opts.set(ShellOption::NoUnset, true);
        let listing = opts.reinputtable_listing();
        assert!(listing.contains("set -o nounset\n"));
        assert!(listing.contains("set +o errexit\n"));
        assert!(!listing.contains("interactive"));
    }

    #[test]
    fn long_listing_names_every_settable_option() {
        let listing = ShellOptions::default().long_listing();
        assert_eq!(
            listing.lines().count(),
            ALL.iter().filter(|s| s.settable()).count()
        );
        assert!(listing.contains("errexit        \toff"));
    }
}

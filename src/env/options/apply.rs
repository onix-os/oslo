//! The grammar of `set`'s arguments: which words are options and which are operands.
//!
//! Kept apart from the `set` builtin so the rule that broke every `set -euo pipefail` script —
//! *every* word became a positional parameter — is unit-testable without a shell to run it in,
//! and so the same walk can serve the command line (`rush -e script`) later.
//!
//! Nothing is applied until the whole argument list has parsed. bash validates first too
//! (`set -e -z` leaves `errexit` *off*), and it is the only defensible answer: a typo in the
//! fourth letter of `set -euxo` must not leave the shell in a state no line of the script asked
//! for.

use super::ShellOption;
use std::fmt;

/// What `set -o` was asked to print.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetListing {
    /// `set -o` with no name: `name<TAB>on|off`.
    Long,
    /// `set +o` with no name: the same states as `set -o name` commands.
    Reinputtable,
}

/// A fully-parsed `set` argument list. Everything here is still pending.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct SetArgs {
    /// Option changes in the order they were written, so `set -e +e` ends with `errexit` off.
    pub changes: Vec<(ShellOption, bool)>,
    /// Listings requested, in order. `set -o -o` prints twice, as bash does.
    pub listings: Vec<SetListing>,
    /// The new positional parameters, or `None` to leave them alone.
    ///
    /// The distinction is the whole point: `set -e` must not wipe `$1`, while `set --` must.
    pub positional: Option<Vec<String>>,
}

/// Why an argument list was refused. Both forms are usage errors, worth exit status 2.
#[derive(Debug, PartialEq, Eq)]
pub enum SetError {
    /// A letter no option uses, or one of the invocation flags `set` may not change.
    InvalidOption(char),
    /// A `-o` operand that names no option.
    InvalidName(String),
}

impl fmt::Display for SetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SetError::InvalidOption(c) => write!(f, "-{}: invalid option", c),
            SetError::InvalidName(name) => write!(f, "{}: invalid option name", name),
        }
    }
}

/// Parse `args[1..]` — `args[0]` is the command word — into a pending [`SetArgs`].
///
/// Option words end at the first operand, at `--`, or at a bare `-`; everything after that is a
/// positional parameter even when it looks like an option, which is what makes
/// `set -- "$@"` and `set -- -x` safe.
pub fn parse_set_args(args: &[String]) -> Result<SetArgs, SetError> {
    let mut parsed = SetArgs::default();
    let mut idx = 1;

    while idx < args.len() {
        let word = args[idx].as_str();

        if word == "--" {
            parsed.positional = Some(args[idx + 1..].to_vec());
            return Ok(parsed);
        }

        // A bare `-` or `+` is the historical form: it turns tracing off and, *only if operands
        // follow*, replaces the positional parameters. `set -e -` leaves `$1` alone.
        if word == "-" || word == "+" {
            parsed.changes.push((ShellOption::XTrace, false));
            parsed.changes.push((ShellOption::Verbose, false));
            let rest = &args[idx + 1..];
            if !rest.is_empty() {
                parsed.positional = Some(rest.to_vec());
            }
            return Ok(parsed);
        }

        let Some(on) = word
            .strip_prefix('-')
            .map(|_| true)
            .or_else(|| word.strip_prefix('+').map(|_| false))
        else {
            parsed.positional = Some(args[idx..].to_vec());
            return Ok(parsed);
        };

        idx = parse_cluster(args, idx, on, &mut parsed)?;
    }

    Ok(parsed)
}

/// Consume one option word, e.g. `-eu` or `+o`, and return the index of the next unread word.
///
/// `-o` is the only letter that takes an argument. It reads the rest of its own word if there is
/// one (`-oerrexit`), otherwise the next word — but only when that word is not itself an option,
/// because `set -o -e` means "list the options, then set errexit" and must not silently name an
/// option `-e`.
fn parse_cluster(
    args: &[String],
    idx: usize,
    on: bool,
    parsed: &mut SetArgs,
) -> Result<usize, SetError> {
    let letters: Vec<char> = args[idx].chars().skip(1).collect();
    let mut pos = 0;

    while pos < letters.len() {
        let letter = letters[pos];
        if letter != 'o' {
            let option = ShellOption::from_letter(letter).ok_or(SetError::InvalidOption(letter))?;
            parsed.changes.push((option, on));
            pos += 1;
            continue;
        }

        let attached: String = letters[pos + 1..].iter().collect();
        if !attached.is_empty() {
            let option = ShellOption::from_name(&attached)
                .ok_or_else(|| SetError::InvalidName(attached.clone()))?;
            parsed.changes.push((option, on));
            return Ok(idx + 1);
        }

        return match args.get(idx + 1) {
            Some(name) if !name.starts_with('-') && !name.starts_with('+') => {
                let option = ShellOption::from_name(name)
                    .ok_or_else(|| SetError::InvalidName(name.clone()))?;
                parsed.changes.push((option, on));
                Ok(idx + 2)
            }
            _ => {
                parsed.listings.push(if on {
                    SetListing::Long
                } else {
                    SetListing::Reinputtable
                });
                Ok(idx + 1)
            }
        };
    }

    Ok(idx + 1)
}

#[cfg(test)]
mod tests {
    use super::{SetError, SetListing, parse_set_args};
    use crate::env::options::ShellOption;

    fn args(parts: &[&str]) -> Vec<String> {
        std::iter::once("set")
            .chain(parts.iter().copied())
            .map(str::to_string)
            .collect()
    }

    fn parse(parts: &[&str]) -> super::SetArgs {
        parse_set_args(&args(parts)).expect("parses")
    }

    /// The finding: `set -euo pipefail` used to set `$1='-euo'`, `$2='pipefail'`.
    #[test]
    fn the_canonical_strict_mode_line_sets_no_positionals() {
        let p = parse(&["-euo", "pipefail"]);
        assert_eq!(
            p.changes,
            vec![
                (ShellOption::ErrExit, true),
                (ShellOption::NoUnset, true),
                (ShellOption::PipeFail, true),
            ]
        );
        assert_eq!(p.positional, None);
        assert!(p.listings.is_empty());
    }

    #[test]
    fn only_words_after_the_options_become_positionals() {
        let p = parse(&["-e", "a", "-b"]);
        assert_eq!(p.changes, vec![(ShellOption::ErrExit, true)]);
        assert_eq!(p.positional, Some(vec!["a".into(), "-b".into()]));
    }

    #[test]
    fn plus_turns_an_option_off() {
        let p = parse(&["+e", "+o", "nounset"]);
        assert_eq!(
            p.changes,
            vec![(ShellOption::ErrExit, false), (ShellOption::NoUnset, false)]
        );
    }

    /// `set --` clears the positionals; `set -e` on its own must not.
    #[test]
    fn double_dash_clears_while_a_bare_option_preserves() {
        assert_eq!(parse(&["--"]).positional, Some(Vec::new()));
        assert_eq!(parse(&["-e"]).positional, None);
        assert_eq!(
            parse(&["--", "-x", "--"]).positional,
            Some(vec!["-x".into(), "--".into()])
        );
    }

    /// A bare `-` turns off tracing and only sets positionals when operands follow it.
    #[test]
    fn bare_dash_is_the_historical_form() {
        let p = parse(&["-"]);
        assert_eq!(
            p.changes,
            vec![(ShellOption::XTrace, false), (ShellOption::Verbose, false)]
        );
        assert_eq!(p.positional, None);
        assert_eq!(parse(&["-", "a"]).positional, Some(vec!["a".into()]));
    }

    #[test]
    fn dash_o_with_no_name_asks_for_a_listing() {
        assert_eq!(parse(&["-o"]).listings, vec![SetListing::Long]);
        assert_eq!(parse(&["+o"]).listings, vec![SetListing::Reinputtable]);
        // The next word is an option, not a name, so it is not eaten by `-o`.
        let p = parse(&["-o", "-e"]);
        assert_eq!(p.listings, vec![SetListing::Long]);
        assert_eq!(p.changes, vec![(ShellOption::ErrExit, true)]);
    }

    #[test]
    fn an_option_name_may_be_attached_to_its_letter() {
        assert_eq!(
            parse(&["-oerrexit"]).changes,
            vec![(ShellOption::ErrExit, true)]
        );
    }

    #[test]
    fn an_unknown_option_is_reported_and_nothing_is_applied() {
        assert_eq!(
            parse_set_args(&args(&["-e", "-z"])).unwrap_err(),
            SetError::InvalidOption('z')
        );
        assert_eq!(
            parse_set_args(&args(&["-o", "badname"])).unwrap_err(),
            SetError::InvalidName("badname".to_string())
        );
        // The invocation flags are not settable: `set -i` must not fake an interactive shell.
        assert_eq!(
            parse_set_args(&args(&["-i"])).unwrap_err(),
            SetError::InvalidOption('i')
        );
    }

    /// An operand that looks like nothing at all still ends the options.
    #[test]
    fn a_leading_operand_ends_option_parsing() {
        let p = parse(&["foo", "-e"]);
        assert!(p.changes.is_empty());
        assert_eq!(p.positional, Some(vec!["foo".into(), "-e".into()]));
    }
}

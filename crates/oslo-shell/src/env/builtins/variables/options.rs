//! Leading-option parsing shared by `export`, `unset`, `local`, `readonly` and `unalias`.
//!
//! Every one of these builtins used to treat `-p` as a *name*, so `export -p` created a variable
//! called `-p` and `unalias -a` deleted an alias called `-a`. One parser, used by all of them,
//! is what stops that class of bug coming back one builtin at a time.

use crate::env::origin_now;
use oslo_base::error::ShellError;

/// The option letters found before the operands, and where the operands start.
#[derive(Debug)]
pub struct Options {
    letters: Vec<char>,
    /// Index into the original `args` of the first operand, i.e. one past the last option word.
    pub operands: usize,
}

impl Options {
    pub fn has(&self, letter: char) -> bool {
        self.letters.contains(&letter)
    }
}

/// Parse the option words of `args[1..]`, accepting only the letters in `accepted`.
///
/// Stops at the first word that is not an option, at an explicit `--`, or at a bare `-`, which
/// POSIX makes an operand rather than an empty option group. `Err` carries the offending letter.
pub fn parse(args: &[String], accepted: &str) -> Result<Options, char> {
    let mut letters = Vec::new();
    let mut operands = args.len();

    for (idx, word) in args.iter().enumerate().skip(1) {
        if word == "--" {
            operands = idx + 1;
            return Ok(Options { letters, operands });
        }
        if !word.starts_with('-') || word.len() == 1 {
            operands = idx;
            return Ok(Options { letters, operands });
        }
        for c in word.chars().skip(1) {
            if !accepted.contains(c) {
                return Err(c);
            }
            if !letters.contains(&c) {
                letters.push(c);
            }
        }
    }

    Ok(Options { letters, operands })
}

/// Report an option the builtin does not accept, and yield the error a caller should return.
///
/// Status 2, not 1: a usage error is not the same as the command running and failing, and scripts
/// that branch on `$?` rely on the difference.
///
/// A [`ShellError::UtilityError`] rather than a bare `2` because POSIX 2.8.1 ends a
/// non-interactive shell whose *special* builtin hit a utility error, and three of this parser's
/// five users are special. Which of them are is not decided here: `crate::exec::simple::posix`
/// asks `is_special_builtin`, so `export -z` ends a `--posix` shell while `unalias -z` reports 2
/// and carries on — matching bash on both. Outside POSIX mode every one of them folds back to
/// `Ok(2)`, which is what this function used to return directly.
pub fn invalid(builtin: &str, letter: char, usage: &str) -> ShellError {
    eprintln!("{}{}: -{}: invalid option", origin_now(), builtin, letter);
    eprintln!("{}", usage);
    ShellError::utility_error(format!("{}: -{}: invalid option", builtin, letter), 2)
}

#[cfg(test)]
mod tests {
    use super::parse;
    use crate::env::builtins::variables::tests::words;

    #[test]
    fn options_stop_at_the_first_operand() {
        let args = words(&["export", "-p", "-n", "FOO", "-x"]);
        let opts = parse(&args, "fnp").expect("all letters accepted");
        assert!(opts.has('p') && opts.has('n'));
        assert_eq!(&args[opts.operands..], &words(&["FOO", "-x"])[..]);
    }

    #[test]
    fn grouped_letters_are_split() {
        let args = words(&["unset", "-fv"]);
        let opts = parse(&args, "fv").expect("both letters accepted");
        assert!(opts.has('f') && opts.has('v'));
        assert_eq!(opts.operands, 2);
    }

    /// `--` ends the options so a name that looks like one can still be operated on.
    #[test]
    fn double_dash_ends_the_options() {
        let args = words(&["unalias", "--", "-a"]);
        let opts = parse(&args, "a").expect("no options before --");
        assert!(!opts.has('a'));
        assert_eq!(&args[opts.operands..], &words(&["-a"])[..]);
    }

    #[test]
    fn a_bare_dash_is_an_operand() {
        let args = words(&["unset", "-"]);
        let opts = parse(&args, "fv").expect("a bare dash is not an option");
        assert_eq!(opts.operands, 1);
    }

    #[test]
    fn an_unaccepted_letter_is_reported() {
        let args = words(&["export", "-z"]);
        assert_eq!(parse(&args, "fnp").unwrap_err(), 'z');
    }
}

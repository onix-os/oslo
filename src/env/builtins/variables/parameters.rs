//! `set` and `shift`: the shell options, the positional parameters, and the state listing.

use super::deparse::function_definition;
use super::quoting::quote_minimal;
use crate::env::options::{SetError, SetListing, parse_set_args};
use crate::env::scope::{Environment, is_valid_identifier};
use crate::error::Result;

const USAGE: &str = "set: usage: set [-abCefhkmnotuvx] [-o option-name] [--] [-] [arg ...]";

/// `set` — change shell options, replace the positional parameters, or list the shell's state.
///
/// The three jobs are one builtin because POSIX made them one, and telling them apart is where
/// this used to go wrong: every word was a positional parameter, so `set -e` armed nothing and
/// `set -euo pipefail` left `$1` as `-euo`. Option words now end at the first operand, at `--`,
/// or at a bare `-`; see [`parse_set_args`] for the grammar.
///
/// The no-argument listing is sorted and quoted. It used to iterate a `HashMap` and print values
/// raw, which meant two runs of the same script produced different output *and* neither could be
/// read back: `x=a b` is an assignment followed by a command.
pub fn builtin_set(env: &mut Environment, args: &[String]) -> Result<i32> {
    if args.len() == 1 {
        print_variables(env);
        print_functions(env);
        return Ok(0);
    }

    // Parsed in full before anything is applied: a bad letter late in the line must not leave
    // half the options changed. bash agrees — `set -e -z` leaves `errexit` off.
    let parsed = match parse_set_args(args) {
        Ok(parsed) => parsed,
        Err(err) => {
            eprintln!("rush: set: {}", err);
            // The usage line answers "which letters are there?", so it follows a bad *letter*
            // and not a bad `-o` name, where it would list nothing relevant.
            if matches!(err, SetError::InvalidOption(_)) {
                eprintln!("{}", USAGE);
            }
            return Ok(2);
        }
    };

    for (option, on) in parsed.changes {
        env.set_option(option, on);
    }
    for listing in parsed.listings {
        let text = match listing {
            SetListing::Long => env.options().long_listing(),
            SetListing::Reinputtable => env.options().reinputtable_listing(),
        };
        print!("{}", text);
    }
    // `None` means no operands were given, which leaves `$1 …` exactly as they were: `set -e`
    // must not wipe the arguments the script was called with.
    if let Some(positional) = parsed.positional {
        env.set_positional(positional);
    }

    Ok(0)
}

/// Print every shell variable as `name=value`, sorted, quoted only where quoting is needed.
///
/// Names `environ` accepted but no shell can parse — bash's `BASH_FUNC_x%%` entries arrive this
/// way — are skipped, because the listing's contract is that a shell can read it back.
fn print_variables(env: &Environment) {
    let vars = env.get_all_vars();
    let mut names: Vec<&String> = vars
        .keys()
        .filter(|name| is_valid_identifier(name))
        .collect();
    names.sort();
    for name in names {
        println!("{}={}", name, quote_minimal(&vars[name]));
    }
}

/// Print every function definition, sorted by name.
fn print_functions(env: &Environment) {
    let functions = env.get_functions();
    let mut names: Vec<&String> = functions.keys().collect();
    names.sort();
    for name in names {
        print!("{}", function_definition(name, &functions[name]));
    }
}

/// `shift [n]` — drop the first `n` positional parameters.
pub fn builtin_shift(env: &mut Environment, args: &[String]) -> Result<i32> {
    let n = if args.len() > 1 {
        match args[1].parse::<usize>() {
            Ok(num) => num,
            Err(_) => {
                eprintln!("rush: shift: {}: numeric argument required", args[1]);
                return Ok(1);
            }
        }
    } else {
        1
    };

    let pos = env.get_positional().to_vec();
    if n > pos.len() {
        eprintln!("rush: shift: shift count out of range");
        return Ok(1);
    }

    env.set_positional(pos[n..].to_vec());
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::builtin_set;
    use crate::env::Environment;
    use crate::env::builtins::variables::tests::words;

    fn set(env: &mut Environment, parts: &[&str]) -> i32 {
        let mut argv = vec!["set".to_string()];
        argv.extend(words(parts));
        builtin_set(env, &argv).expect("set never errors")
    }

    /// The finding: every one of these words used to become a positional parameter.
    #[test]
    fn options_are_options_and_not_positionals() {
        let mut env = Environment::new();
        env.set_positional(words(&["keep", "me"]));
        assert_eq!(set(&mut env, &["-euo", "pipefail"]), 0);
        assert!(env.errexit() && env.nounset() && env.pipefail());
        assert_eq!(env.get_positional(), &words(&["keep", "me"])[..]);
        assert_eq!(env.get_param("-").as_deref(), Some("eu"));
    }

    #[test]
    fn operands_after_the_options_are_the_new_positionals() {
        let mut env = Environment::new();
        assert_eq!(set(&mut env, &["-x", "a", "b"]), 0);
        assert!(env.xtrace());
        assert_eq!(env.get_positional(), &words(&["a", "b"])[..]);

        // `--` protects an operand that looks like an option.
        assert_eq!(set(&mut env, &["--", "-e"]), 0);
        assert_eq!(env.get_positional(), &words(&["-e"])[..]);
        assert!(!env.errexit());
    }

    #[test]
    fn plus_clears_an_option() {
        let mut env = Environment::new();
        set(&mut env, &["-e"]);
        assert!(env.errexit());
        set(&mut env, &["+e"]);
        assert!(!env.errexit());
        set(&mut env, &["-o", "noclobber"]);
        assert!(env.noclobber());
        set(&mut env, &["+o", "noclobber"]);
        assert!(!env.noclobber());
    }

    #[test]
    fn an_unknown_option_is_a_usage_error_worth_two() {
        let mut env = Environment::new();
        assert_eq!(set(&mut env, &["-e", "-z"]), 2);
        // Nothing was applied: the whole line is refused, not the part after the mistake.
        assert!(!env.errexit());
        assert_eq!(set(&mut env, &["-o", "nosuchthing"]), 2);
        assert_eq!(set(&mut env, &["-i"]), 2);
    }

    #[test]
    fn set_with_no_arguments_still_lists_variables() {
        let mut env = Environment::new();
        env.set_positional(words(&["a"]));
        assert_eq!(set(&mut env, &[]), 0);
        assert_eq!(env.get_positional(), &words(&["a"])[..]);
    }
}

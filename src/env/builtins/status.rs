//! `status` — what kind of shell this is, and where in it you are.
//!
//! ```text
//! status is-interactive      is there somebody typing at this?
//! status is-login            was it started as a login shell?
//! status current-function    the innermost function, or nothing
//! status function            the whole call stack, innermost first
//! status oslo-path           the path to this shell
//! status basename            the name it was invoked as
//! ```
//!
//! fish's, minus the parts that describe fish. The reason it is worth having is the first line of
//! every dotfile repo in existence:
//!
//! ```sh
//! status is-interactive || return
//! ```
//!
//! Without it, the portable spelling is `case $- in *i*) … esac` — correct, and not something
//! anybody remembers. oslo already knew every one of these facts; there was simply no way to ask.
//!
//! **Predicates answer through the exit status**, so they compose with `&&` and `||` rather than
//! needing to be compared against a string. Everything else prints and succeeds.

use crate::env::Environment;
use crate::error::Result;

pub fn builtin_status(env: &mut Environment, args: &[String]) -> Result<i32> {
    let Some(subcommand) = args.get(1) else {
        // Bare `status` prints the two facts that decide what a config does, rather than being a
        // usage error. Nothing to parse, and it is what you want when you are asking by hand.
        println!(
            "interactive: {}\nlogin: {}",
            yes_no(env.interactive()),
            yes_no(is_login(env))
        );
        return Ok(0);
    };

    Ok(match subcommand.as_str() {
        "--help" | "-h" => {
            usage();
            0
        }
        // The predicates. `0` is true, as it is for `test`.
        "is-interactive" => status_of(env.interactive()),
        "is-login" => status_of(is_login(env)),
        "is-block" | "is-function" => status_of(!env.call_stack().is_empty()),

        "current-function" => match env.call_stack().last() {
            Some(name) => {
                println!("{name}");
                0
            }
            // Silent, like `caller` outside a function: a script testing this reads the status.
            None => 1,
        },
        "function" | "stack-trace" => {
            let frames = env.call_stack();
            if frames.is_empty() {
                return Ok(1);
            }
            // Innermost first, which is the order a stack trace is read in and the order
            // `current-function` agrees with.
            for name in frames.iter().rev() {
                println!("{name}");
            }
            0
        }

        "oslo-path" | "shell-path" => {
            match std::env::current_exe() {
                Ok(path) => println!("{}", path.display()),
                // `$0` is the fallback and not an error: a shell whose own path cannot be read is
                // still a shell, and this is the answer a script would have used anyway.
                Err(_) => println!("{}", env.shell_name),
            }
            0
        }
        "basename" => {
            println!("{}", basename(&env.shell_name));
            0
        }

        other => {
            eprintln!("status: {other}: unknown subcommand");
            usage();
            2
        }
    })
}

/// Whether this is a login shell, spelled by convention with a leading `-` in `$0`.
///
/// The `-` is the only marker there is: `su -`, `login(1)` and every display manager set it, and
/// there is no second source to cross-check it against. `suspend` reads it the same way.
fn is_login(env: &Environment) -> bool {
    env.shell_name.starts_with('-')
}

/// The invoked name without its directory, and without the login `-`.
fn basename(name: &str) -> &str {
    let bare = name.strip_prefix('-').unwrap_or(name);
    bare.rsplit('/').next().unwrap_or(bare)
}

/// A predicate's answer as an exit status: true is `0`, the way `test` has it.
fn status_of(answer: bool) -> i32 {
    i32::from(!answer)
}

fn yes_no(answer: bool) -> &'static str {
    if answer { "yes" } else { "no" }
}

fn usage() {
    println!("Usage: status [SUBCOMMAND]");
    println!();
    println!("  is-interactive     succeed if a person is typing at this shell");
    println!("  is-login           succeed if it was started as a login shell");
    println!("  is-function        succeed inside a shell function");
    println!("  current-function   print the innermost function's name");
    println!("  function           print the call stack, innermost first");
    println!("  oslo-path          print the path to this shell");
    println!("  basename           print the name it was invoked as");
    println!();
    println!("The predicates answer through the exit status, so `status is-interactive || return`");
    println!("works. With no subcommand, the interactive and login facts are printed.");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env::options::ShellOption;

    fn argv(words: &[&str]) -> Vec<String> {
        std::iter::once("status".to_string())
            .chain(words.iter().map(|w| w.to_string()))
            .collect()
    }

    fn run(env: &mut Environment, words: &[&str]) -> i32 {
        builtin_status(env, &argv(words)).expect("status never fails fatally")
    }

    /// The line every dotfile repo opens with. A non-interactive shell must answer 1, or
    /// `status is-interactive || return` runs the interactive half of every config in every script.
    #[test]
    fn is_interactive_follows_the_invocation() {
        let mut env = Environment::new();
        assert_eq!(run(&mut env, &["is-interactive"]), 1, "a script is not");
        env.set_option(ShellOption::Interactive, true);
        assert_eq!(run(&mut env, &["is-interactive"]), 0);
    }

    /// A login shell is spelled with a leading `-` in `$0`, which is the convention `su -` and
    /// every display manager uses.
    #[test]
    fn a_login_shell_is_recognised_by_its_name() {
        let mut env = Environment::new();
        env.shell_name = "oslo".to_string();
        assert_eq!(run(&mut env, &["is-login"]), 1);
        env.shell_name = "-oslo".to_string();
        assert_eq!(run(&mut env, &["is-login"]), 0);
    }

    /// Inside a function the name is available; outside it the status says so rather than printing
    /// an empty line, so `f=$(status current-function) || …` can tell the cases apart.
    #[test]
    fn the_current_function_is_the_innermost_one() {
        let mut env = Environment::new();
        assert_eq!(run(&mut env, &["current-function"]), 1, "not in one");
        assert_eq!(run(&mut env, &["is-function"]), 1);

        env.enter_function_named("outer").expect("depth");
        env.enter_function_named("inner").expect("depth");
        assert_eq!(run(&mut env, &["current-function"]), 0);
        assert_eq!(env.call_stack().last().map(String::as_str), Some("inner"));
        assert_eq!(run(&mut env, &["is-function"]), 0);
    }

    /// The invoked name, without its directory and without the login dash.
    #[test]
    fn the_basename_strips_the_path_and_the_login_dash() {
        assert_eq!(basename("/usr/bin/oslo"), "oslo");
        assert_eq!(basename("-oslo"), "oslo");
        assert_eq!(basename("-/usr/bin/oslo"), "oslo");
        assert_eq!(basename("oslo"), "oslo");
    }

    /// An unknown subcommand is a usage error, not a silent success — a config branching on a
    /// name oslo does not know must not take the wrong branch.
    #[test]
    fn an_unknown_subcommand_is_a_usage_error() {
        let mut env = Environment::new();
        assert_eq!(run(&mut env, &["is-fish"]), 2);
        // Bare `status` reports rather than failing.
        assert_eq!(run(&mut env, &[]), 0);
        assert_eq!(run(&mut env, &["--help"]), 0);
    }
}

//! Command-line argument handling for the `rush` binary.
//!
//! Kept apart from `main` so the decision table — which option takes an argument, what becomes
//! `$0`, when the shell is interactive — can be unit-tested without spawning a process.
//!
//! The rule that matters most: an argument this parser does not understand is an error. The
//! previous implementation recognised three forms and silently started a REPL for everything
//! else, so `rush --version` read the caller's stdin and exited 0.

use rush::env::options::ShellOption;
use std::fmt::Write as _;

/// Where the shell's input comes from.
#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    /// `-c COMMAND`: run this program text, then exit.
    Command(String),
    /// A script operand: read the program from this path.
    Script(String),
    /// `--lua-script FILE`: run a Lua configuration script instead of a shell script.
    LuaScript(String),
    /// `-s`, or no operand at all: read the program from standard input.
    Stdin,
}

/// A fully-understood invocation.
#[derive(Debug, PartialEq, Eq)]
pub struct Invocation {
    pub action: Action,
    /// The value of `$0`.
    pub name: String,
    /// Positional parameters `$1`, `$2`, …
    pub positional: Vec<String>,
    /// `-i`: be interactive even when stdin is not a terminal.
    pub force_interactive: bool,
    /// `-l`: behave as a login shell.
    pub login: bool,
    /// Single-letter `set` options given on the command line, e.g. `ex` for `-e -x`.
    ///
    /// Letters only, in the order they were written, deduplicated. `main` turns each one into a
    /// [`ShellOption`] on the new shell's `Environment`; the letters that are not options at all
    /// (`-c`, `-i`, `-l`, `-s`) are handled by the fields above and never appear here.
    pub set_options: String,
}

/// Argument parsing finished without producing an invocation: print `message` and exit.
#[derive(Debug, PartialEq, Eq)]
pub struct Exit {
    pub message: String,
    pub to_stderr: bool,
    pub status: i32,
}

pub fn version_line() -> String {
    format!("rush version {}", env!("CARGO_PKG_VERSION"))
}

pub fn usage() -> String {
    let mut s = String::new();
    let _ = writeln!(s, "usage: rush [option]... [script [argument]...]");
    let _ = writeln!(s, "       rush [option]... -c command [name [argument]...]");
    let _ = writeln!(s);
    let _ = writeln!(s, "Options:");
    let _ = writeln!(s, "  -c COMMAND        run COMMAND, then exit");
    let _ = writeln!(s, "  -s                read commands from standard input");
    let _ = writeln!(s, "  -i                force interactive mode");
    let _ = writeln!(s, "  -l                act as a login shell");
    let _ = writeln!(
        s,
        "  -e -u -x ...      set a shell option, as `set` does (see `set -o`)"
    );
    let _ = writeln!(s, "  --lua-script FILE run a Lua script, then exit");
    let _ = writeln!(s, "  --version         print the version, then exit");
    let _ = writeln!(s, "  --help            print this message, then exit");
    let _ = writeln!(s, "  --                end of options");
    s
}

fn usage_error(problem: String) -> Exit {
    Exit {
        message: format!("rush: {}\n{}", problem, usage()),
        to_stderr: true,
        status: 2,
    }
}

/// Interpret `argv` (including `argv[0]`).
pub fn parse(argv: &[String]) -> Result<Invocation, Exit> {
    let mut name = argv.first().cloned().unwrap_or_else(|| "rush".to_string());
    let mut command: Option<String> = None;
    let mut lua_script: Option<String> = None;
    let mut read_stdin = false;
    let mut force_interactive = false;
    let mut login = false;
    let mut set_options = String::new();

    let mut i = 1;
    while i < argv.len() {
        let arg = argv[i].clone();

        // `--` and a bare `-` both end option processing; neither is an operand.
        if arg == "--" {
            i += 1;
            break;
        }
        if arg == "-" {
            i += 1;
            break;
        }

        if let Some(long) = arg.strip_prefix("--") {
            match long {
                "version" => {
                    return Err(Exit {
                        message: version_line(),
                        to_stderr: false,
                        status: 0,
                    });
                }
                "help" => {
                    return Err(Exit {
                        message: usage().trim_end().to_string(),
                        to_stderr: false,
                        status: 0,
                    });
                }
                "lua-script" => {
                    i += 1;
                    match argv.get(i) {
                        Some(path) => lua_script = Some(path.clone()),
                        None => {
                            return Err(usage_error(
                                "--lua-script: option requires an argument".to_string(),
                            ));
                        }
                    }
                }
                other => {
                    return Err(usage_error(format!("--{}: invalid option", other)));
                }
            }
            i += 1;
            continue;
        }

        if !arg.starts_with('-') {
            break; // an operand: the script name, or `-c`'s `$0`
        }

        // A cluster of single-letter options, e.g. `-ex`. `-c` consumes whatever follows it in
        // the cluster, or the next argument when the cluster ends there — `-c'echo hi'` and
        // `-c 'echo hi'` mean the same thing.
        let letters: Vec<char> = arg.chars().skip(1).collect();
        let mut pos = 0;
        while pos < letters.len() {
            match letters[pos] {
                'c' => {
                    let rest: String = letters[pos + 1..].iter().collect();
                    if rest.is_empty() {
                        i += 1;
                        match argv.get(i) {
                            Some(text) => command = Some(text.clone()),
                            None => {
                                return Err(usage_error(
                                    "-c: option requires an argument".to_string(),
                                ));
                            }
                        }
                    } else {
                        command = Some(rest);
                    }
                    pos = letters.len();
                    continue;
                }
                's' => read_stdin = true,
                'i' => force_interactive = true,
                'l' => login = true,
                // Any letter `set` would accept means the same thing here, so
                // `rush -f script.sh` starts with globbing off rather than being rejected.
                // The table in `rush::env::options` is the only list of them.
                letter if ShellOption::from_letter(letter).is_some() => {
                    if !set_options.contains(letter) {
                        set_options.push(letter);
                    }
                }
                other => {
                    return Err(usage_error(format!("-{}: invalid option", other)));
                }
            }
            pos += 1;
        }
        i += 1;
    }

    let operands = &argv[i.min(argv.len())..];

    // Precedence: an explicit `--lua-script` or `-c` decides where the program comes from;
    // `-s` forces stdin even when operands follow; otherwise the first operand is a script.
    let (action, positional) = match (lua_script, command) {
        (Some(path), _) => (Action::LuaScript(path), operands.to_vec()),
        // With `-c`, the first operand is `$0` and the rest are positional, as in POSIX.
        (None, Some(text)) => {
            if let Some((zero, rest)) = operands.split_first() {
                name = zero.clone();
                (Action::Command(text), rest.to_vec())
            } else {
                (Action::Command(text), Vec::new())
            }
        }
        (None, None) if read_stdin => (Action::Stdin, operands.to_vec()),
        (None, None) => match operands.split_first() {
            Some((path, rest)) => {
                name = path.clone();
                (Action::Script(path.clone()), rest.to_vec())
            }
            None => (Action::Stdin, Vec::new()),
        },
    };

    Ok(Invocation {
        action,
        name,
        positional,
        force_interactive,
        login,
        set_options,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_args(args: &[&str]) -> Result<Invocation, Exit> {
        let argv: Vec<String> = std::iter::once("rush")
            .chain(args.iter().copied())
            .map(str::to_string)
            .collect();
        parse(&argv)
    }

    #[test]
    fn no_arguments_reads_stdin() {
        let inv = parse_args(&[]).expect("parse");
        assert_eq!(inv.action, Action::Stdin);
        assert_eq!(inv.name, "rush");
        assert!(inv.positional.is_empty());
    }

    #[test]
    fn dash_c_takes_the_next_argument() {
        let inv = parse_args(&["-c", "echo hi"]).expect("parse");
        assert_eq!(inv.action, Action::Command("echo hi".to_string()));
    }

    #[test]
    fn dash_c_takes_an_attached_argument() {
        let inv = parse_args(&["-cecho hi"]).expect("parse");
        assert_eq!(inv.action, Action::Command("echo hi".to_string()));
    }

    #[test]
    fn dash_c_without_an_argument_is_a_usage_error() {
        let err = parse_args(&["-c"]).expect_err("must not start a REPL");
        assert_eq!(err.status, 2);
        assert!(err.to_stderr);
        assert!(
            err.message.contains("requires an argument"),
            "{}",
            err.message
        );
    }

    #[test]
    fn dash_c_operands_supply_zero_and_positionals() {
        let inv = parse_args(&["-c", "echo", "myname", "a", "b"]).expect("parse");
        assert_eq!(inv.name, "myname");
        assert_eq!(inv.positional, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn script_operand_becomes_zero_and_the_rest_positional() {
        let inv = parse_args(&["run.sh", "one", "two"]).expect("parse");
        assert_eq!(inv.action, Action::Script("run.sh".to_string()));
        assert_eq!(inv.name, "run.sh");
        assert_eq!(inv.positional, vec!["one".to_string(), "two".to_string()]);
    }

    #[test]
    fn clustered_options_are_all_seen() {
        let inv = parse_args(&["-eix", "run.sh"]).expect("parse");
        assert!(inv.force_interactive);
        assert_eq!(inv.set_options, "ex");
        assert_eq!(inv.action, Action::Script("run.sh".to_string()));
    }

    #[test]
    fn double_dash_ends_options() {
        let inv = parse_args(&["--", "-weird-name.sh"]).expect("parse");
        assert_eq!(inv.action, Action::Script("-weird-name.sh".to_string()));
    }

    #[test]
    fn unknown_short_option_is_a_usage_error() {
        let err = parse_args(&["-Z"]).expect_err("unknown option");
        assert_eq!(err.status, 2);
        assert!(
            err.message.contains("-Z: invalid option"),
            "{}",
            err.message
        );
    }

    #[test]
    fn unknown_long_option_is_a_usage_error() {
        let err = parse_args(&["--nope"]).expect_err("unknown option");
        assert_eq!(err.status, 2);
        assert!(
            err.message.contains("--nope: invalid option"),
            "{}",
            err.message
        );
    }

    #[test]
    fn version_and_help_exit_zero_on_stdout() {
        let v = parse_args(&["--version"]).expect_err("terminates");
        assert_eq!(v.status, 0);
        assert!(!v.to_stderr);
        assert!(v.message.contains(env!("CARGO_PKG_VERSION")));

        let h = parse_args(&["--help"]).expect_err("terminates");
        assert_eq!(h.status, 0);
        assert!(!h.to_stderr);
        assert!(h.message.contains("usage: rush"));
    }

    #[test]
    fn dash_s_reads_stdin_with_positionals() {
        let inv = parse_args(&["-s", "a", "b"]).expect("parse");
        assert_eq!(inv.action, Action::Stdin);
        assert_eq!(inv.positional, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn lua_script_still_works() {
        let inv = parse_args(&["--lua-script", "init.lua"]).expect("parse");
        assert_eq!(inv.action, Action::LuaScript("init.lua".to_string()));
    }
}

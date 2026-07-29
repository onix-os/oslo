//! Command-line argument handling for the `oslo` binary.
//!
//! Kept apart from `main` so the decision table — which option takes an argument, what becomes
//! `$0`, when the shell is interactive — can be unit-tested without spawning a process.
//!
//! The rule that matters most: an argument this parser does not understand is an error. The
//! previous implementation recognised three forms and silently started a REPL for everything
//! else, so `oslo --version` read the caller's stdin and exited 0.

use crate::startup::language::Language;
use oslo::env::options::ShellOption;
use std::fmt::Write as _;

/// Where the shell's input comes from.
#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    /// `-c COMMAND`: run this program text, then exit.
    Command(String),
    /// A script operand: read the program from this path.
    Script(String),
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
    /// Letters only, in the order they were written, deduplicated. The letters that are not
    /// options at all (`-c`, `-i`, `-l`, `-s`) are handled by the fields above and never appear
    /// here. Read through [`Invocation::options`], never directly: an option with no letter
    /// cannot be spelled here at all.
    pub set_options: String,
    /// `--lua` or `--sh`: run the program as that language instead of detecting it.
    ///
    /// `None` is the normal case and means "work it out from the shebang, the extension, then the
    /// text" — see [`crate::startup::language`]. The flag exists for the file that genuinely
    /// cannot be told apart, not as the usual way to run Lua.
    pub force_language: Option<Language>,
    /// Options given by their long name, e.g. `--posix`.
    ///
    /// A separate field because [`Self::set_options`] is a string of letters and the options that
    /// matter most here — `posix` above all — deliberately have none: `$-` must not claim a
    /// letter bash does not report.
    pub long_options: Vec<ShellOption>,
}

impl Invocation {
    /// Every `set` option this command line asks for, however it was spelled.
    ///
    /// The one place a caller should read the two option fields from, so that adding a long
    /// option can never again mean adding a second loop somewhere that forgets it.
    pub fn options(&self) -> impl Iterator<Item = ShellOption> + '_ {
        self.set_options
            .chars()
            .filter_map(ShellOption::from_letter)
            .chain(self.long_options.iter().copied())
    }
}

/// Argument parsing finished without producing an invocation: print `message` and exit.
#[derive(Debug, PartialEq, Eq)]
pub struct Exit {
    pub message: String,
    pub to_stderr: bool,
    pub status: i32,
}

pub fn version_line() -> String {
    format!("oslo version {}", env!("CARGO_PKG_VERSION"))
}

pub fn usage() -> String {
    let mut s = String::new();
    let _ = writeln!(s, "usage: oslo [option]... [script [argument]...]");
    let _ = writeln!(s, "       oslo [option]... -c command [name [argument]...]");
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
    let _ = writeln!(
        s,
        "  --posix           follow POSIX where bash's default differs"
    );
    let _ = writeln!(
        s,
        "  --lua             run the program as Lua (normally detected)"
    );
    let _ = writeln!(
        s,
        "  --sh              run the program as shell (normally detected)"
    );
    let _ = writeln!(s, "  --version         print the version, then exit");
    let _ = writeln!(s, "  --help            print this message, then exit");
    let _ = writeln!(s, "  --                end of options");
    s
}

/// The `set -o` names that are also command-line flags.
///
/// Deliberately a short explicit list rather than "anything `set -o` accepts": bash rejects
/// `--errexit`, and a shell that quietly accepted it would be inventing an interface. `--posix`
/// is here because POSIX mode cannot be reached any other way before the first command runs —
/// `set -o posix` on line 1 of a script is already too late for the option to have decided how
/// that line's command word was searched for.
const LONG_FLAGS: &[&str] = &["posix"];

fn long_option(name: &str) -> Option<ShellOption> {
    LONG_FLAGS
        .contains(&name)
        .then(|| ShellOption::from_name(name))
        .flatten()
}

fn usage_error(problem: String) -> Exit {
    Exit {
        message: format!("oslo: {}\n{}", problem, usage()),
        to_stderr: true,
        status: 2,
    }
}

/// Interpret `argv` (including `argv[0]`).
pub fn parse(argv: &[String]) -> Result<Invocation, Exit> {
    let mut name = argv.first().cloned().unwrap_or_else(|| "oslo".to_string());
    let mut command: Option<String> = None;
    let mut force_language: Option<Language> = None;
    let mut read_stdin = false;
    let mut force_interactive = false;
    let mut login = false;
    let mut set_options = String::new();
    let mut long_options: Vec<ShellOption> = Vec::new();

    let mut i = 1;
    // Set once `-c` has taken its command string: everything after it is an operand, whatever it
    // looks like. bash, dash and the Debian `sh` all agree — `sh -c 'echo $1' -- a` puts `--` in
    // `$0` rather than treating it as the end of options, and `sh -c cmd -x y` runs with `-x` as
    // `$0` and tracing *off*. It matters because `find -exec sh -c '…' -- {} +` and the `xargs`
    // idioms built on it are exactly how the `-c` convention is used (PLAN R9.12).
    let mut operands_only = false;

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
                "lua" => force_language = Some(Language::Lua),
                "sh" => force_language = Some(Language::Shell),
                other => match long_option(other) {
                    Some(option) => {
                        if !long_options.contains(&option) {
                            long_options.push(option);
                        }
                    }
                    None => return Err(usage_error(format!("--{}: invalid option", other))),
                },
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
                    operands_only = true;
                    pos = letters.len();
                    continue;
                }
                's' => read_stdin = true,
                'i' => force_interactive = true,
                'l' => login = true,
                // Any letter `set` would accept means the same thing here, so
                // `oslo -f script.sh` starts with globbing off rather than being rejected.
                // The table in `oslo::env::options` is the only list of them.
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
        if operands_only {
            break;
        }
    }

    let operands = &argv[i.min(argv.len())..];

    // Precedence: `-c` decides where the program comes from; `-s` forces stdin even when operands
    // follow; otherwise the first operand is a script. Which *language* that program is written in
    // is a separate question, answered after the text is in hand.
    let (action, positional) = match command {
        // With `-c`, the first operand is `$0` and the rest are positional, as in POSIX.
        Some(text) => {
            if let Some((zero, rest)) = operands.split_first() {
                name = zero.clone();
                (Action::Command(text), rest.to_vec())
            } else {
                (Action::Command(text), Vec::new())
            }
        }
        None if read_stdin => (Action::Stdin, operands.to_vec()),
        None => match operands.split_first() {
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
        force_language,
        long_options,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_args(args: &[&str]) -> Result<Invocation, Exit> {
        let argv: Vec<String> = std::iter::once("oslo")
            .chain(args.iter().copied())
            .map(str::to_string)
            .collect();
        parse(&argv)
    }

    #[test]
    fn no_arguments_reads_stdin() {
        let inv = parse_args(&[]).expect("parse");
        assert_eq!(inv.action, Action::Stdin);
        assert_eq!(inv.name, "oslo");
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
    fn dash_c_ends_option_parsing_at_its_command_string() {
        // `find -exec sh -c '…' -- {} +` passes `--` as `$0`; treating it as the end of options
        // would shift every positional by one and silently run the command against nothing.
        let inv = parse_args(&["-c", "echo", "--", "x", "y"]).expect("parse");
        assert_eq!(inv.name, "--");
        assert_eq!(inv.positional, vec!["x".to_string(), "y".to_string()]);

        // Nor is a later `-x` an option any more: it is `$1`, and tracing stays off.
        let inv = parse_args(&["-c", "echo", "name", "-x", "z"]).expect("parse");
        assert_eq!(inv.name, "name");
        assert_eq!(inv.positional, vec!["-x".to_string(), "z".to_string()]);
        assert_eq!(inv.set_options, "");
    }

    #[test]
    fn options_before_dash_c_are_still_options() {
        let inv = parse_args(&["-x", "-c", "echo", "name"]).expect("parse");
        assert_eq!(inv.set_options, "x");
        assert_eq!(inv.name, "name");
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
        assert!(h.message.contains("usage: oslo"));
    }

    #[test]
    fn dash_s_reads_stdin_with_positionals() {
        let inv = parse_args(&["-s", "a", "b"]).expect("parse");
        assert_eq!(inv.action, Action::Stdin);
        assert_eq!(inv.positional, vec!["a".to_string(), "b".to_string()]);
    }

    /// `--posix` was not an option at all: `oslo --posix -c 'echo x'` was a usage error, which is
    /// what made the whole POSIX-mode code path unreachable in production.
    #[test]
    fn posix_is_a_long_option_and_reaches_the_option_set() {
        let inv = parse_args(&["--posix", "-c", "echo x"]).expect("parse");
        assert_eq!(inv.action, Action::Command("echo x".to_string()));
        assert!(inv.options().any(|o| o == ShellOption::Posix));
        // It has no letter, so it must not have leaked into the `$-` string either.
        assert_eq!(inv.set_options, "");
    }

    /// `options()` is the single reader, so a mixed command line yields both spellings.
    #[test]
    fn options_yields_letters_and_long_names_together() {
        let inv = parse_args(&["--posix", "-ex", "run.sh"]).expect("parse");
        let opts: Vec<_> = inv.options().collect();
        assert!(opts.contains(&ShellOption::ErrExit));
        assert!(opts.contains(&ShellOption::XTrace));
        assert!(opts.contains(&ShellOption::Posix));
    }

    /// Repeats collapse, as they do for letters.
    #[test]
    fn a_repeated_long_option_is_recorded_once() {
        let inv = parse_args(&["--posix", "--posix"]).expect("parse");
        assert_eq!(inv.long_options, vec![ShellOption::Posix]);
    }

    /// The long-name flags are an explicit list, not "everything `set -o` accepts": bash rejects
    /// `--errexit`, and accepting it would be inventing an interface.
    #[test]
    fn a_set_o_name_is_not_automatically_a_long_option() {
        let err = parse_args(&["--errexit"]).expect_err("not a command-line flag");
        assert!(
            err.message.contains("--errexit: invalid option"),
            "{}",
            err.message
        );
    }

    /// A Lua file is an ordinary operand. There is no flag to run Lua, which is the point:
    /// `--lua-script FILE` used to be the only way, in a shell whose scripting language is Lua.
    #[test]
    fn a_lua_file_is_just_a_script_operand() {
        let inv = parse_args(&["init.lua"]).expect("parse");
        assert_eq!(inv.action, Action::Script("init.lua".to_string()));
        assert_eq!(inv.name, "init.lua");
        assert_eq!(inv.force_language, None, "nothing was forced");
    }

    #[test]
    fn the_language_can_still_be_forced_either_way() {
        let lua = parse_args(&["--lua", "script"]).expect("parse");
        assert_eq!(lua.force_language, Some(Language::Lua));
        assert_eq!(lua.action, Action::Script("script".to_string()));

        let sh = parse_args(&["--sh", "script"]).expect("parse");
        assert_eq!(sh.force_language, Some(Language::Shell));
    }

    /// The flag it replaced must not linger as a silently-accepted no-op.
    #[test]
    fn the_old_lua_script_flag_is_gone() {
        let err = parse_args(&["--lua-script", "init.lua"]).expect_err("must be rejected");
        assert!(
            err.message.contains("--lua-script: invalid option"),
            "{}",
            err.message
        );
    }
}

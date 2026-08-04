//! Command-line argument handling for the `oslo` binary.
//!
//! Kept apart from `main` so the decision table — which option takes an argument, what becomes
//! `$0`, when the shell is interactive — can be unit-tested without spawning a process.
//!
//! The rule that matters most: an argument this parser does not understand is an error. The
//! previous implementation recognised three forms and silently started a REPL for everything
//! else, so `oslo --version` read the caller's stdin and exited 0.

pub mod help;
pub mod tools;

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
    /// `oslo history …`: one of oslo's own tools, named in the operand slot.
    ///
    /// The name rather than the [`tools::Tool`] so that this type stays comparable and printable
    /// for the parser's tests; the binary looks it up again to run it.
    Tool(String, Vec<String>),
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
    /// `--vi` / `--no-vi`: force the line editor's mode, overriding `oslo.vi.enabled`.
    ///
    /// `None` — the normal case — leaves the config in charge.
    pub vi: Option<bool>,
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

/// Printing the help is not a failure, so it goes to stdout with status 0 — which is also what
/// makes `oslo --help | less` and `oslo --help > FILE` work, and both of those are why the paint
/// is detected rather than assumed.
fn help_exit(detailed: bool) -> Exit {
    let paint = help::Paint::detect();
    let text = if detailed {
        help::details(paint)
    } else {
        help::short(paint)
    };
    Exit {
        message: text.trim_end().to_string(),
        to_stderr: false,
        status: 0,
    }
}

/// The two-line synopsis a usage *error* prints.
///
/// Short on purpose. A mistyped flag should say what was wrong and how to ask for more, not bury
/// it under the whole reference — `oslo --help` is one keystroke away and is where the detail is.
pub fn usage() -> String {
    let mut s = String::new();
    let _ = writeln!(s, "usage: oslo [option]... [script [argument]...]");
    let _ = writeln!(s, "       oslo [option]... -c command [name [argument]...]");
    let _ = write!(s, "try `oslo --help` for the options.");
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
/// Whether being called by this name means "be a POSIX shell".
///
/// **`sh` is a personality, not just a path.** bash has done this since 1989 — invoked as `sh` it
/// enters POSIX mode, invoked as `bash` it does not, and the same binary serves both. Verified
/// against the real bash rather than taken from the manual: `ln -s bash sh; ./sh -c 'echo
/// $SHELLOPTS'` lists `posix`; `bash -c` on the identical binary does not.
///
/// It matters most for the case oslo is built for. A distro that points `/bin/sh` at oslo is
/// asking for a POSIX shell, and every `#!/bin/sh` script on it then gets one without a flag
/// anybody has to remember — which is the only way a system-wide default can actually hold.
///
/// The leading `-` of a login shell is stripped first: `su -` and every display manager start
/// `/bin/sh` as `-sh`, and those are the shells this matters for most.
fn named_sh(argv0: &str) -> bool {
    let base = argv0.rsplit('/').next().unwrap_or(argv0);
    base.strip_prefix('-').unwrap_or(base) == "sh"
}

pub fn parse(argv: &[String]) -> Result<Invocation, Exit> {
    let mut name = argv.first().cloned().unwrap_or_else(|| "oslo".to_string());
    let mut command: Option<String> = None;
    let mut force_language: Option<Language> = None;
    let mut vi: Option<bool> = None;
    let mut read_stdin = false;
    let mut ended_options = false;
    let mut force_interactive = false;
    let mut login = false;
    let mut set_options = String::new();
    let mut long_options: Vec<ShellOption> = Vec::new();

    // Before the flags are read, so `--posix` on top of it is simply the same option twice.
    if argv.first().is_some_and(|argv0| named_sh(argv0)) {
        long_options.push(ShellOption::Posix);
    }

    let mut i = 1;
    // Set once `-c` has taken its command string: everything after it is an operand, whatever it
    // looks like. bash, dash and the Debian `sh` all agree — `sh -c 'echo $1' -- a` puts `--` in
    // `$0` rather than treating it as the end of options, and `sh -c cmd -x y` runs with `-x` as
    // `$0` and tracing *off*. It matters because `find -exec sh -c '…' -- {} +` and the `xargs`
    // idioms built on it are exactly how the `-c` convention is used (PLAN R9.12).
    let mut operands_only = false;

    while i < argv.len() {
        let arg = argv[i].clone();

        // `--` and a bare `-` both end option processing; neither is an operand. Remembered,
        // because `--` is also how somebody says "the next word is a path, not a tool name".
        if arg == "--" || arg == "-" {
            ended_options = true;
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
                // `--details` modifies `--help` and may be written on either side of it, so the
                // rest of the command line is consulted rather than only what has been seen. It
                // stops at `--`, past which a `--details` is somebody's argument and not a flag.
                // `--details` modifies `--help` and may be written on either side of it, so the
                // rest of the command line is consulted rather than only what has been seen. It
                // stops at `--`, past which a `--details` is somebody's argument and not a flag.
                "help" => {
                    let detailed = argv[i + 1..]
                        .iter()
                        .take_while(|arg| *arg != "--")
                        .any(|arg| arg == "--details");
                    return Err(help_exit(detailed));
                }
                // On its own it means the same thing, so neither spelling has to be remembered.
                "details" => return Err(help_exit(true)),
                // `--login` beside `-l`. bash accepts both, and the long form is what a display
                // manager, a terminal emulator's "run as login shell" setting and `su --login`
                // reach for — so a shell that only took `-l` failed to start under exactly the
                // things that start a login shell.
                "login" => login = true,
                "lua" => force_language = Some(Language::Lua),
                "sh" => force_language = Some(Language::Shell),
                // **Off only.** vi bindings are oslo's default, so there is nothing for a `--vi`
                // to turn on from the command line. Both spellings, because the editing mode is
                // vi and everyone calls the editor vim.
                "no-vi" | "no-vim" => vi = Some(false),
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
            // A tool only when nothing else could have been meant — see `tools::as_operand`. Not
            // after an explicit `--`, which is how you say "this really is a path".
            Some((word, rest)) if !ended_options && tools::as_operand(word).is_some() => {
                name = word.clone();
                (Action::Tool(word.clone(), rest.to_vec()), Vec::new())
            }
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
        vi,
        long_options,
    })
}

#[cfg(test)]
#[path = "cli/tests.rs"]
mod tests;

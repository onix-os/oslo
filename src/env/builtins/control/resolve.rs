//! `type` — report how the shell would resolve a name.
//!
//! The order here *is* the dispatch order: alias, reserved word, function, builtin, `$PATH`.
//! `type` that disagrees with what the shell actually runs is worse than no `type` at all, so
//! this list is the one place to change if dispatch changes.

use super::format_function;
use crate::ast::Command;
use crate::env::scope::Environment;
use crate::error::Result;
use std::path::PathBuf;

/// Reserved words: recognised by the parser, so they never reach command dispatch.
///
/// Not read from the parser because it has no such table to read — the grammar matches them
/// inline. Kept here rather than in `Environment` because `type` is the only thing that asks.
const KEYWORDS: &[&str] = &[
    "!", "[[", "]]", "case", "coproc", "do", "done", "elif", "else", "esac", "fi", "for",
    "function", "if", "in", "select", "then", "time", "until", "while", "{", "}",
];

/// One way a name resolves, in the order dispatch would try them.
enum Kind {
    Alias(String),
    Keyword,
    Function(Command),
    Builtin,
    File(PathBuf),
}

impl Kind {
    /// The word `type -t` prints.
    fn terse(&self) -> &'static str {
        match self {
            Kind::Alias(_) => "alias",
            Kind::Keyword => "keyword",
            Kind::Function(_) => "function",
            Kind::Builtin => "builtin",
            Kind::File(_) => "file",
        }
    }

    fn describe(&self, name: &str) -> String {
        match self {
            Kind::Alias(value) => format!("{name} is aliased to `{value}'"),
            Kind::Keyword => format!("{name} is a shell keyword"),
            Kind::Function(body) => {
                format!("{name} is a function\n{}", format_function(name, body))
            }
            Kind::Builtin => format!("{name} is a shell builtin"),
            Kind::File(path) => format!("{name} is {}", path.display()),
        }
    }
}

#[derive(Default)]
struct Options {
    /// `-a`: every match, not just the first.
    all: bool,
    /// `-t`: the one-word kind instead of a sentence.
    terse: bool,
    /// `-p`: the path, but only for a name that resolves to a file.
    path: bool,
    /// `-P`: the path from a forced `$PATH` search, whatever else the name is.
    force_path: bool,
    /// `-f`: skip the function lookup, as `command` does.
    no_functions: bool,
}

/// Split the leading option run off `args`, or produce the exit status for a usage error.
fn parse_options(args: &[String]) -> std::result::Result<(Options, &[String]), i32> {
    let mut opts = Options::default();
    let mut rest = args;
    while let Some(arg) = rest.first() {
        // `--` ends the options; a lone `-` and anything not starting with `-` is a name.
        if arg == "--" {
            rest = &rest[1..];
            break;
        }
        if !arg.starts_with('-') || arg == "-" {
            break;
        }
        for c in arg.chars().skip(1) {
            match c {
                'a' => opts.all = true,
                't' => opts.terse = true,
                'p' => opts.path = true,
                'P' => opts.force_path = true,
                'f' => opts.no_functions = true,
                other => {
                    eprintln!("rush: type: -{other}: invalid option");
                    eprintln!("type: usage: type [-afptP] name [name ...]");
                    return Err(2);
                }
            }
        }
        rest = &rest[1..];
    }
    Ok((opts, rest))
}

/// Every executable named `name` on `$PATH`, in search order.
///
/// `which_all` rather than `which` because `type -a` has to show the shadowed ones too — that is
/// most of the reason anyone runs it.
fn path_matches(name: &str, all: bool) -> Vec<PathBuf> {
    if all {
        which::which_all(name)
            .map(|found| found.collect())
            .unwrap_or_default()
    } else {
        which::which(name).into_iter().collect()
    }
}

fn resolve(env: &Environment, name: &str, opts: &Options) -> Vec<Kind> {
    let mut kinds = Vec::new();
    if opts.force_path {
        // `-P` asks what `$PATH` alone would find, ignoring every shell-level name.
        kinds.extend(path_matches(name, opts.all).into_iter().map(Kind::File));
        return kinds;
    }

    if let Some(value) = env.get_alias(name) {
        kinds.push(Kind::Alias(value.to_string()));
    }
    if KEYWORDS.contains(&name) {
        kinds.push(Kind::Keyword);
    }
    if !opts.no_functions
        && let Some(body) = env.get_function(name)
    {
        kinds.push(Kind::Function(body.clone()));
    }
    if env.is_builtin(name) {
        kinds.push(Kind::Builtin);
    }
    // Without `-a` a single earlier match settles it, and a `$PATH` walk would be wasted work.
    if opts.all || kinds.is_empty() {
        kinds.extend(path_matches(name, opts.all).into_iter().map(Kind::File));
    }
    kinds
}

/// `type [-afptP] name …`
pub fn builtin_type(env: &mut Environment, args: &[String]) -> Result<i32> {
    let (opts, names) = match parse_options(args.get(1..).unwrap_or_default()) {
        Ok(parsed) => parsed,
        Err(code) => return Ok(code),
    };

    let mut status = 0;
    for name in names {
        let kinds = resolve(env, name, &opts);
        let shown: &[Kind] = if opts.all {
            &kinds
        } else {
            kinds.get(..1).unwrap_or_default()
        };

        if shown.is_empty() {
            // `-t`, `-p` and `-P` are meant to be read by scripts: they report "no" with an exit
            // status and stay silent, where bare `type` complains.
            if !opts.terse && !opts.path && !opts.force_path {
                eprintln!("rush: type: {name}: not found");
            }
            status = 1;
            continue;
        }

        for kind in shown {
            if opts.terse {
                println!("{}", kind.terse());
            } else if opts.path || opts.force_path {
                // `-p` prints nothing for a name that is not a file; it is not an error.
                if let Kind::File(path) = kind {
                    println!("{}", path.display());
                }
            } else {
                println!("{}", kind.describe(name));
            }
        }
    }

    Ok(status)
}

#[cfg(test)]
mod tests {
    use super::{Kind, builtin_type, parse_options, resolve};
    use crate::ast::Command;
    use crate::env::scope::Environment;

    fn args(words: &[&str]) -> Vec<String> {
        words.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn options_stop_at_double_dash_and_at_the_first_name() {
        let argv = args(["-at", "--", "-t"].as_slice());
        let (opts, names) = parse_options(&argv).expect("valid options");
        assert!(opts.all && opts.terse);
        assert_eq!(names, ["-t".to_string()]);

        let argv = args(["ls", "-t"].as_slice());
        let (_, names) = parse_options(&argv).expect("valid options");
        assert_eq!(names.len(), 2, "options do not follow the first name");
    }

    /// The flags used to be looked up as if they were command names, so `type -t f` reported
    /// `-t: not found`.
    #[test]
    fn an_unknown_option_is_a_usage_error() {
        assert_eq!(parse_options(&args(["-z"].as_slice())).err(), Some(2));
    }

    #[test]
    fn a_lone_dash_is_a_name_not_an_option() {
        let argv = args(["-"].as_slice());
        let (_, names) = parse_options(&argv).expect("valid options");
        assert_eq!(names, ["-".to_string()]);
    }

    #[test]
    fn resolution_order_puts_alias_and_function_ahead_of_the_builtin() {
        let mut env = Environment::new();
        crate::env::builtins::register_default_builtins(&mut env);
        env.set_alias("cd", "cd -P");
        let parsed = crate::parser::parse_bash_script("cd() { :; }").expect("parses");
        let Some(Command::FunctionDef { body, .. }) =
            parsed.items[0].and_or.first.commands.first().cloned()
        else {
            panic!("expected a function definition");
        };
        env.set_function("cd", *body);

        let opts = super::Options {
            all: true,
            ..Default::default()
        };
        let kinds = resolve(&env, "cd", &opts);
        let terse: Vec<&str> = kinds.iter().map(Kind::terse).collect();
        assert_eq!(&terse[..3], ["alias", "function", "builtin"]);
    }

    #[test]
    fn keywords_are_reported_as_keywords() {
        let mut env = Environment::new();
        crate::env::builtins::register_default_builtins(&mut env);
        let kinds = resolve(&env, "if", &super::Options::default());
        assert_eq!(kinds.iter().map(Kind::terse).next(), Some("keyword"));
    }

    #[test]
    fn a_missing_name_is_status_one() {
        let mut env = Environment::new();
        crate::env::builtins::register_default_builtins(&mut env);
        let argv = args(["type", "definitely-not-a-command-xyzzy"].as_slice());
        assert_eq!(builtin_type(&mut env, &argv).expect("no error"), 1);
    }
}

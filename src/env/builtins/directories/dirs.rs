//! `dirs`: how the directory stack is shown.
//!
//! Presentation only — the model lives in `stack`. The one wrinkle is that the default listing
//! abbreviates `$HOME` to `~`, which `-l` turns off; a script that wants to *use* the paths
//! wants `dirs -l`, and one that wants to show them to a person wants the default.

use super::chdir::logical_pwd;
use super::stack::{is_index, resolve_index, stack, store};
use crate::env::scope::Environment;
use crate::error::Result;

const DIRS_USAGE: &str = "dirs: usage: dirs [-clpv] [+N] [-N]";

/// How the entries are laid out.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Layout {
    /// All on one line, separated by blanks. The default.
    Line,
    /// One per line.
    Lines,
    /// One per line, prefixed with its stack index.
    Numbered,
}

#[derive(Debug)]
struct Options {
    clear: bool,
    long: bool,
    layout: Layout,
    index: Option<String>,
}

/// `$HOME` shortened to `~`, unless `-l` asked for the path in full.
fn present(env: &Environment, path: &str, long: bool) -> String {
    if long {
        return path.to_string();
    }
    match env.get_var("HOME") {
        Some(home) if !home.is_empty() && path == home => "~".to_string(),
        Some(home) if !home.is_empty() && path.starts_with(&format!("{home}/")) => {
            format!("~{}", &path[home.len()..])
        }
        _ => path.to_string(),
    }
}

fn parse_options(args: &[String]) -> std::result::Result<Options, i32> {
    let mut options = Options {
        clear: false,
        long: false,
        layout: Layout::Line,
        index: None,
    };
    for arg in args.iter().skip(1) {
        if is_index(arg) {
            options.index = Some(arg.clone());
            continue;
        }
        let Some(flags) = arg.strip_prefix('-').filter(|rest| !rest.is_empty()) else {
            eprintln!("rush: dirs: {arg}: invalid number");
            eprintln!("{DIRS_USAGE}");
            return Err(2);
        };
        for flag in flags.chars() {
            match flag {
                'c' => options.clear = true,
                'l' => options.long = true,
                'p' => options.layout = Layout::Lines,
                // `-v` is `-p` plus the indices, so it wins over a preceding `-p`.
                'v' => options.layout = Layout::Numbered,
                other => {
                    eprintln!("rush: dirs: -{other}: invalid number");
                    eprintln!("{DIRS_USAGE}");
                    return Err(2);
                }
            }
        }
    }
    Ok(options)
}

pub fn builtin_dirs(env: &mut Environment, args: &[String]) -> Result<i32> {
    let options = match parse_options(args) {
        Ok(options) => options,
        Err(status) => return Ok(status),
    };

    if options.clear {
        // Only the pushed entries are dropped: entry 0 is the directory the shell is in, and
        // `dirs -c` does not move it.
        let current = logical_pwd(env);
        store(env, &[current]);
        return Ok(0);
    }

    let entries = stack(env);

    if let Some(spec) = options.index {
        let Some(index) = resolve_index(&spec, entries.len()) else {
            eprintln!("rush: dirs: directory stack empty");
            return Ok(1);
        };
        println!("{}", present(env, &entries[index], options.long));
        return Ok(0);
    }

    match options.layout {
        Layout::Line => {
            let shown: Vec<String> = entries
                .iter()
                .map(|entry| present(env, entry, options.long))
                .collect();
            println!("{}", shown.join(" "));
        }
        Layout::Lines => {
            for entry in &entries {
                println!("{}", present(env, entry, options.long));
            }
        }
        Layout::Numbered => {
            for (index, entry) in entries.iter().enumerate() {
                // Two columns of index, matching bash, so the paths line up until the stack is
                // a hundred deep.
                println!("{:2}  {}", index, present(env, entry, options.long));
            }
        }
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_is_abbreviated_unless_long() {
        let mut env = Environment::new();
        env.set_var("HOME", "/home/u", false);
        assert_eq!(present(&env, "/home/u", false), "~");
        assert_eq!(present(&env, "/home/u/sub", false), "~/sub");
        assert_eq!(present(&env, "/home/underscore", false), "/home/underscore");
        assert_eq!(present(&env, "/home/u/sub", true), "/home/u/sub");
    }

    #[test]
    fn flags_may_be_bundled() {
        let args: Vec<String> = ["dirs", "-lv"].iter().map(|s| s.to_string()).collect();
        let options = parse_options(&args).expect("flags parse");
        assert!(options.long);
        assert_eq!(options.layout, Layout::Numbered);
    }

    #[test]
    fn an_unknown_flag_is_a_usage_error() {
        let args: Vec<String> = ["dirs", "-x"].iter().map(|s| s.to_string()).collect();
        assert_eq!(parse_options(&args).unwrap_err(), 2);
    }
}

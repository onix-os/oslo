//! `cd` and `pwd`: the option matrix in front of the shared change-directory helper.

use super::chdir::{PathMode, change_directory, logical_pwd};
use crate::env::scope::Environment;
use crate::error::Result;
use std::env;

const CD_USAGE: &str = "cd: usage: cd [-L|-P] [dir]";
const PWD_USAGE: &str = "pwd: usage: pwd [-LP]";

/// Consume a leading run of `-L`/`-P` flags and an optional `--`, returning the mode and the
/// operands that follow.
///
/// Last flag wins (`cd -LP` is `cd -P`), a bare `-` is an operand rather than an option, and the
/// first operand ends option parsing — so `cd dir -P` has two operands, which is an error, not a
/// physical `cd`.
fn parse_mode(args: &[String]) -> std::result::Result<(PathMode, &[String]), String> {
    let mut mode = PathMode::Logical;
    let mut index = 1;
    while index < args.len() {
        let arg = args[index].as_str();
        if arg == "--" {
            index += 1;
            break;
        }
        if arg.len() < 2 || !arg.starts_with('-') {
            break;
        }
        // `cd -3` is a destination, not a mistyped option: it means three back through the
        // directory history. Stopping here leaves it to be read as an operand, where it belongs.
        if arg[1..].chars().all(|c| c.is_ascii_digit()) {
            break;
        }
        for flag in arg.chars().skip(1) {
            match flag {
                'L' => mode = PathMode::Logical,
                'P' => mode = PathMode::Physical,
                other => return Err(format!("-{other}")),
            }
        }
        index += 1;
    }
    Ok((mode, &args[index..]))
}

pub fn builtin_cd(env: &mut Environment, args: &[String]) -> Result<i32> {
    let (mode, operands) = match parse_mode(args) {
        Ok(parsed) => parsed,
        Err(flag) => {
            eprintln!("oslo: cd: {flag}: invalid option");
            eprintln!("{CD_USAGE}");
            return Ok(2);
        }
    };

    if operands.len() > 1 {
        // A usage error rather than a failed cd: the shell has not moved, and bash reports 2
        // for every builtin whose arguments do not parse.
        eprintln!("oslo: cd: too many arguments");
        return Ok(2);
    }

    // `cd -` announces where it went, because the destination came from the environment rather
    // than from anything visible in the script.
    let mut announce = false;
    let target = match operands.first().map(String::as_str) {
        None => match env.get_var("HOME").map(str::to_string) {
            Some(home) if !home.is_empty() => home,
            _ => {
                eprintln!("oslo: cd: HOME not set");
                return Ok(1);
            }
        },
        Some("-") => {
            announce = true;
            match env.get_var("OLDPWD").map(str::to_string) {
                Some(old) if !old.is_empty() => old,
                _ => {
                    eprintln!("oslo: cd: OLDPWD not set");
                    return Ok(1);
                }
            }
        }
        // `cd -3` — three directories back through the ring, which `cd -` cannot express and
        // which is what you want the moment you are more than one wrong turn from home. Only a
        // *number* is treated this way; `cd -L` and `cd -P` are options and were handled above.
        Some(operand)
            if operand.len() > 1
                && operand.starts_with('-')
                && operand[1..].chars().all(|c| c.is_ascii_digit()) =>
        {
            announce = true;
            let n: usize = operand[1..].parse().unwrap_or(0);
            match super::ring::nth_back(n) {
                Some(path) => path,
                None => {
                    eprintln!("oslo: cd: {operand}: no such entry in the directory history");
                    return Ok(1);
                }
            }
        }
        Some(operand) => operand.to_string(),
    };

    match change_directory(env, &target, mode, "cd") {
        Some(destination) => {
            if announce {
                println!("{destination}");
            }
            // Remembered *after* the move succeeded, so a failed `cd` does not appear in the
            // history of places you have been.
            super::ring::record(&destination);
            Ok(0)
        }
        None => Ok(1),
    }
}

pub fn builtin_pwd(env: &mut Environment, args: &[String]) -> Result<i32> {
    let (mode, _operands) = match parse_mode(args) {
        // Operands are ignored: `pwd` has none, and bash does not complain about extras.
        Ok(parsed) => parsed,
        Err(flag) => {
            eprintln!("oslo: pwd: {flag}: invalid option");
            eprintln!("{PWD_USAGE}");
            return Ok(2);
        }
    };

    match mode {
        PathMode::Logical => println!("{}", logical_pwd(env)),
        PathMode::Physical => match env::current_dir() {
            Ok(path) => println!("{}", path.display()),
            Err(e) => {
                eprintln!("oslo: pwd: error retrieving current directory: {e}");
                return Ok(1);
            }
        },
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn last_mode_flag_wins() {
        let args = words(&["cd", "-L", "-P", "dir"]);
        let (mode, operands) = parse_mode(&args).expect("flags parse");
        assert_eq!(mode, PathMode::Physical);
        assert_eq!(operands, ["dir"]);

        let args = words(&["cd", "-PL"]);
        let (mode, operands) = parse_mode(&args).expect("flags parse");
        assert_eq!(mode, PathMode::Logical);
        assert!(operands.is_empty());
    }

    #[test]
    fn double_dash_ends_options() {
        let args = words(&["cd", "--", "-P"]);
        let (mode, operands) = parse_mode(&args).expect("flags parse");
        assert_eq!(mode, PathMode::Logical);
        assert_eq!(operands, ["-P"]);
    }

    #[test]
    fn a_bare_dash_is_an_operand() {
        let args = words(&["cd", "-"]);
        let (_, operands) = parse_mode(&args).expect("flags parse");
        assert_eq!(operands, ["-"]);
    }

    #[test]
    fn an_operand_ends_option_parsing() {
        let args = words(&["cd", "dir", "-P"]);
        let (mode, operands) = parse_mode(&args).expect("flags parse");
        assert_eq!(mode, PathMode::Logical);
        assert_eq!(operands, ["dir", "-P"]);
    }

    #[test]
    fn unknown_flag_is_reported_by_letter() {
        assert_eq!(parse_mode(&words(&["cd", "-x"])).unwrap_err(), "-x");
        assert_eq!(parse_mode(&words(&["cd", "-Lx"])).unwrap_err(), "-x");
    }
}

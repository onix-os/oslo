//! Conditional expressions: the `test` / `[` builtin and the extended `[[` form.

use crate::env::scope::Environment;
use crate::error::Result;
use std::fs;

pub fn builtin_test(_env: &mut Environment, args: &[String]) -> Result<i32> {
    let mut expr_args = &args[1..];
    if args.is_empty() {
        return Ok(1);
    }
    if args[0] == "[" {
        if expr_args.last().map(|s| s.as_str()) == Some("]") {
            expr_args = &expr_args[..expr_args.len() - 1];
        } else {
            eprintln!("rush: [: missing `]'");
            return Ok(2);
        }
    }

    if expr_args.is_empty() {
        return Ok(1);
    }

    if expr_args.len() == 1 {
        return Ok(if expr_args[0].is_empty() { 1 } else { 0 });
    }

    if expr_args.len() == 2 {
        let op = expr_args[0].as_str();
        let target = expr_args[1].as_str();
        let res = match op {
            "-f" => std::path::Path::new(target).is_file(),
            "-d" => std::path::Path::new(target).is_dir(),
            "-e" | "-a" => std::path::Path::new(target).exists(),
            "-z" => target.is_empty(),
            "-n" => !target.is_empty(),
            "-r" => fs::metadata(target).is_ok(),
            "!" => target.is_empty(),
            _ => false,
        };
        return Ok(if res { 0 } else { 1 });
    }

    if expr_args.len() == 3 {
        let left = expr_args[0].as_str();
        let op = expr_args[1].as_str();
        let right = expr_args[2].as_str();

        let res = match op {
            "=" | "==" => left == right,
            "!=" => left != right,
            "-eq" => left.parse::<i64>().unwrap_or(0) == right.parse::<i64>().unwrap_or(0),
            "-ne" => left.parse::<i64>().unwrap_or(0) != right.parse::<i64>().unwrap_or(0),
            "-gt" => left.parse::<i64>().unwrap_or(0) > right.parse::<i64>().unwrap_or(0),
            "-ge" => left.parse::<i64>().unwrap_or(0) >= right.parse::<i64>().unwrap_or(0),
            "-lt" => left.parse::<i64>().unwrap_or(0) < right.parse::<i64>().unwrap_or(0),
            "-le" => left.parse::<i64>().unwrap_or(0) <= right.parse::<i64>().unwrap_or(0),
            _ => false,
        };
        return Ok(if res { 0 } else { 1 });
    }

    Ok(0)
}

/// `[[ ... ]]` — the extended test.
///
/// Never written by hand at this level: [`crate::parser::brush_adapter`] converts the parsed
/// expression tree into these calls, using the shell's own `&&`/`||`/`!` for the connectives so
/// this only has to evaluate a single predicate.
///
/// The difference from [`builtin_test`] that matters is `==`: inside `[[ ]]` an unquoted
/// right-hand side is a glob *pattern*, so `[[ abc == a* ]]` is true. The adapter picks `==`
/// (pattern) or `=` (literal) based on whether the operand was quoted in the source.
pub fn builtin_extended_test(env: &mut Environment, args: &[String]) -> Result<i32> {
    // Strip the `[[` / `]]` bookends the adapter adds.
    let mut expr = &args[1..];
    if expr.last().map(String::as_str) == Some("]]") {
        expr = &expr[..expr.len() - 1];
    }

    let truth = match expr.len() {
        0 => false,
        1 => !expr[0].is_empty(),
        2 => eval_unary(env, &expr[0], &expr[1])?,
        3 => eval_binary(&expr[0], &expr[1], &expr[2])?,
        _ => {
            eprintln!("rush: [[: too many arguments");
            return Ok(2);
        }
    };

    Ok(if truth { 0 } else { 1 })
}

fn eval_unary(env: &Environment, op: &str, target: &str) -> Result<bool> {
    use std::os::unix::fs::FileTypeExt;
    let path = std::path::Path::new(target);

    // File-type predicates need the type bits; absent file means false, not an error.
    let file_type = fs::metadata(path).ok().map(|m| m.file_type());

    Ok(match op {
        "-e" | "-a" => path.exists(),
        "-f" => path.is_file(),
        "-d" => path.is_dir(),
        "-L" => fs::symlink_metadata(path)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false),
        "-s" => fs::metadata(path).map(|m| m.len() > 0).unwrap_or(false),
        "-r" => access(path, nix::unistd::AccessFlags::R_OK),
        "-w" => access(path, nix::unistd::AccessFlags::W_OK),
        "-x" => access(path, nix::unistd::AccessFlags::X_OK),
        "-p" => file_type.is_some_and(|t| t.is_fifo()),
        "-S" => file_type.is_some_and(|t| t.is_socket()),
        "-b" => file_type.is_some_and(|t| t.is_block_device()),
        "-c" => file_type.is_some_and(|t| t.is_char_device()),
        "-z" => target.is_empty(),
        "-n" => !target.is_empty(),
        "-v" => env.get_param(target).is_some(),
        other => {
            eprintln!("rush: [[: {}: unsupported unary operator", other);
            return Ok(false);
        }
    })
}

/// Real access check, rather than inferring readability from a successful `stat`.
fn access(path: &std::path::Path, mode: nix::unistd::AccessFlags) -> bool {
    nix::unistd::access(path, mode).is_ok()
}

fn eval_binary(left: &str, op: &str, right: &str) -> Result<bool> {
    // Arithmetic comparisons: non-numeric operands are an error in bash, but rush's `test`
    // already treats them as 0, so stay consistent with it.
    let nums = || {
        (
            left.trim().parse::<i64>().unwrap_or(0),
            right.trim().parse::<i64>().unwrap_or(0),
        )
    };

    Ok(match op {
        // Literal comparison — the operand was quoted in the source.
        "=" => left == right,
        // Glob comparison — the operand was unquoted.
        "==" => match glob::Pattern::new(right) {
            Ok(p) => p.matches(left),
            // An invalid pattern falls back to literal comparison, matching bash's behaviour of
            // treating an unparseable pattern as ordinary text.
            Err(_) => left == right,
        },
        "<" => left < right,
        ">" => left > right,
        "-eq" => nums().0 == nums().1,
        "-ne" => nums().0 != nums().1,
        "-lt" => nums().0 < nums().1,
        "-le" => nums().0 <= nums().1,
        "-gt" => nums().0 > nums().1,
        "-ge" => nums().0 >= nums().1,
        "-nt" => newer_than(left, right),
        "-ot" => newer_than(right, left),
        "-ef" => same_file(left, right),
        other => {
            eprintln!("rush: [[: {}: unsupported binary operator", other);
            return Ok(false);
        }
    })
}

/// True when `a` exists and is newer than `b`, or when `b` does not exist.
fn newer_than(a: &str, b: &str) -> bool {
    let ma = fs::metadata(a).ok().and_then(|m| m.modified().ok());
    let mb = fs::metadata(b).ok().and_then(|m| m.modified().ok());
    match (ma, mb) {
        (Some(a), Some(b)) => a > b,
        (Some(_), None) => true,
        _ => false,
    }
}

/// True when both paths refer to the same device and inode.
fn same_file(a: &str, b: &str) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (fs::metadata(a), fs::metadata(b)) {
        (Ok(x), Ok(y)) => x.dev() == y.dev() && x.ino() == y.ino(),
        _ => false,
    }
}

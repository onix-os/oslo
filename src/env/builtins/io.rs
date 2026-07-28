//! Reading and writing: `echo` and `read`.

use crate::env::scope::Environment;
use crate::error::Result;
use std::io::{self, BufRead};

pub fn builtin_echo(_env: &mut Environment, args: &[String]) -> Result<i32> {
    let mut print_newline = true;
    let mut start_idx = 1;

    if args.len() > 1 && args[1] == "-n" {
        print_newline = false;
        start_idx = 2;
    }

    let mut output = args[start_idx..].join(" ");
    if print_newline {
        output.push('\n');
    }

    let _ = nix::unistd::write(
        unsafe { std::os::fd::BorrowedFd::borrow_raw(1) },
        output.as_bytes(),
    );

    Ok(0)
}

pub fn builtin_read(env: &mut Environment, args: &[String]) -> Result<i32> {
    if args.len() < 2 {
        return Ok(0);
    }

    let mut line = String::new();
    let stdin = io::stdin();
    if stdin.lock().read_line(&mut line).is_err() {
        return Ok(1);
    }

    let trimmed = line.trim_end();
    let parts: Vec<&str> = trimmed.split_whitespace().collect();

    let var_names = &args[1..];
    for (i, name) in var_names.iter().enumerate() {
        if i == var_names.len() - 1 {
            let rest = parts.get(i..).unwrap_or_default().join(" ");
            env.set_var(name, &rest, false);
        } else {
            let val = parts.get(i).copied().unwrap_or("");
            env.set_var(name, val, false);
        }
    }

    Ok(0)
}

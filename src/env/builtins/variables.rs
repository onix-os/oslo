//! Variables and aliases: `export`, `unset`, `set`, `shift`, `local`, `readonly`,
//! `alias`, `unalias`.

use crate::env::scope::Environment;
use crate::error::Result;

pub fn builtin_export(env: &mut Environment, args: &[String]) -> Result<i32> {
    if args.len() == 1 {
        for (k, v) in env.get_exported_vars() {
            println!("export {}={:?}", k, v);
        }
        return Ok(0);
    }

    for arg in &args[1..] {
        let arg_trimmed = arg.trim();
        if let Some(idx) = arg_trimmed.find('=') {
            let k = arg_trimmed[..idx]
                .trim()
                .trim_matches('\'')
                .trim_matches('"');
            let mut v = arg_trimmed[idx + 1..].trim();
            if ((v.starts_with('"') && v.ends_with('"'))
                || (v.starts_with('\'') && v.ends_with('\'')))
                && v.len() >= 2
            {
                v = &v[1..v.len() - 1];
            }
            env.set_var(k, v, true);
        } else {
            let var_name = arg_trimmed.trim_matches('\'').trim_matches('"');
            env.export_var(var_name);
        }
    }

    Ok(0)
}

pub fn builtin_unset(env: &mut Environment, args: &[String]) -> Result<i32> {
    for arg in &args[1..] {
        env.unset_var(arg);
    }
    Ok(0)
}

pub fn builtin_set(env: &mut Environment, args: &[String]) -> Result<i32> {
    if args.len() == 1 {
        for (k, v) in env.get_all_vars() {
            println!("{}={}", k, v);
        }
        return Ok(0);
    }

    if args.len() > 1 && args[1] == "--" {
        env.set_positional(args[2..].to_vec());
    } else {
        env.set_positional(args[1..].to_vec());
    }

    Ok(0)
}

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

pub fn builtin_alias(env: &mut Environment, args: &[String]) -> Result<i32> {
    if args.len() == 1 {
        return Ok(0);
    }

    for arg in &args[1..] {
        if let Some(idx) = arg.find('=') {
            let k = &arg[..idx];
            let v = &arg[idx + 1..];
            env.set_alias(k, v);
        } else if let Some(val) = env.get_alias(arg) {
            println!("alias {}='{}'", arg, val);
        } else {
            eprintln!("rush: alias: {}: not found", arg);
            return Ok(1);
        }
    }

    Ok(0)
}

pub fn builtin_unalias(env: &mut Environment, args: &[String]) -> Result<i32> {
    for arg in &args[1..] {
        env.remove_alias(arg);
    }
    Ok(0)
}

pub fn builtin_local(env: &mut Environment, args: &[String]) -> Result<i32> {
    for arg in &args[1..] {
        if let Some(idx) = arg.find('=') {
            let k = &arg[..idx];
            let v = &arg[idx + 1..];
            env.set_local_var(k, v);
        } else {
            env.set_local_var(arg, "");
        }
    }
    Ok(0)
}

pub fn builtin_readonly(env: &mut Environment, args: &[String]) -> Result<i32> {
    if args.len() < 2 {
        for (k, _) in env.get_all_vars() {
            if env.is_readonly(&k) {
                println!("readonly {}", k);
            }
        }
        return Ok(0);
    }

    for arg in &args[1..] {
        if let Some(idx) = arg.find('=') {
            let k = &arg[..idx];
            let v = &arg[idx + 1..];
            env.set_var(k, v, false);
            env.set_readonly(k);
        } else {
            env.set_readonly(arg);
        }
    }
    Ok(0)
}

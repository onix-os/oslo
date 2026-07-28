//! Variables and aliases: `export`, `unset`, `set`, `shift`, `local`, `readonly`,
//! `alias`, `unalias`.

use crate::env::scope::{Environment, is_valid_identifier};
use crate::error::Result;

/// Complain about `word` the way bash does for a name that is not `[A-Za-z_][A-Za-z0-9_]*`.
///
/// The whole word is quoted back, not the part before the `=`, so `export '=1'` names what the
/// user actually typed.
fn not_an_identifier(builtin: &str, word: &str) {
    eprintln!("rush: {}: '{}': not a valid identifier", builtin, word);
}

pub fn builtin_export(env: &mut Environment, args: &[String]) -> Result<i32> {
    if args.len() == 1 {
        for (k, v) in env.get_exported_vars() {
            println!("export {}={:?}", k, v);
        }
        return Ok(0);
    }

    let mut status = 0;
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
            if !is_valid_identifier(k) {
                not_an_identifier("export", arg_trimmed);
                status = 1;
            } else if !env.set_var(k, v, true) {
                status = 1;
            }
        } else {
            let var_name = arg_trimmed.trim_matches('\'').trim_matches('"');
            if !is_valid_identifier(var_name) {
                not_an_identifier("export", arg_trimmed);
                status = 1;
            } else if !env.export_var(var_name) {
                status = 1;
            }
        }
    }

    Ok(status)
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
    let mut status = 0;
    for arg in &args[1..] {
        let (k, v) = match arg.find('=') {
            Some(idx) => (&arg[..idx], &arg[idx + 1..]),
            None => (arg.as_str(), ""),
        };
        if !is_valid_identifier(k) {
            not_an_identifier("local", arg);
            status = 1;
        } else if !env.set_local_var(k, v) {
            status = 1;
        }
    }
    Ok(status)
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

    let mut status = 0;
    for arg in &args[1..] {
        let (k, assigned) = match arg.find('=') {
            Some(idx) => (&arg[..idx], Some(&arg[idx + 1..])),
            None => (arg.as_str(), None),
        };
        if !is_valid_identifier(k) {
            not_an_identifier("readonly", arg);
            status = 1;
            continue;
        }
        // A refused assignment must not leave the name read-only: the user would then be unable
        // to set it at all, with nothing to show for it.
        if let Some(v) = assigned
            && !env.set_var(k, v, false)
        {
            status = 1;
            continue;
        }
        env.set_readonly(k);
    }
    Ok(status)
}

#[cfg(test)]
mod tests {
    use super::{builtin_export, builtin_local, builtin_readonly};
    use crate::env::scope::Environment;

    fn words(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| s.to_string()).collect()
    }

    /// Before R1.7 this call reached `env::set_var("", "1")` and killed the process.
    #[test]
    fn export_of_an_invalid_name_fails_without_setting_anything() {
        let mut env = Environment::new();
        for bad in ["=1", "1abc=x", "a b=1", "a-b"] {
            assert_eq!(
                builtin_export(&mut env, &words(&["export", bad])).unwrap(),
                1,
                "export {bad:?} should fail"
            );
        }
        assert!(env.get_var("=1").is_none());
        assert!(env.get_var("1abc").is_none());
    }

    /// A value carrying a NUL — from `read` over a binary file, say — cannot go into `environ`.
    #[test]
    fn export_of_a_nul_bearing_value_fails() {
        let mut env = Environment::new();
        assert_eq!(
            builtin_export(&mut env, &words(&["export", "NUL_VAR=a\0b"])).unwrap(),
            1
        );
        assert!(env.get_var("NUL_VAR").is_none());
    }

    /// One bad name must not stop the good names on the same line, but must still set status 1.
    ///
    /// Tested through `local` rather than `export` so the assertion never writes to the real
    /// `environ`: these unit tests run on parallel threads, and mutating `environ` under them is
    /// exactly the hazard the `unsafe` blocks in `scope.rs` are documented against.
    #[test]
    fn a_bad_name_does_not_stop_the_rest_of_the_line() {
        let mut env = Environment::new();
        env.push_scope();
        let args = words(&["local", "=1", "GOOD_ONE=yes"]);
        assert_eq!(builtin_local(&mut env, &args).unwrap(), 1);
        assert_eq!(env.get_var("GOOD_ONE"), Some("yes"));
        env.pop_scope();
    }

    #[test]
    fn local_of_an_invalid_name_fails() {
        let mut env = Environment::new();
        env.push_scope();
        assert_eq!(
            builtin_local(&mut env, &words(&["local", "=1"])).unwrap(),
            1
        );
        assert_eq!(builtin_local(&mut env, &words(&["local", "x"])).unwrap(), 0);
        // Popping is where a name smuggled into the frame would have aborted.
        env.pop_scope();
    }

    #[test]
    fn readonly_of_an_invalid_name_fails() {
        let mut env = Environment::new();
        assert_eq!(
            builtin_readonly(&mut env, &words(&["readonly", "=1"])).unwrap(),
            1
        );
        assert!(!env.is_readonly("=1"));
        assert!(!env.is_readonly(""));
    }

    /// A refused assignment must not leave the name read-only, or the variable becomes
    /// permanently unusable for no reason the user can see.
    #[test]
    fn readonly_with_a_rejected_value_does_not_lock_the_name() {
        let mut env = Environment::new();
        assert_eq!(
            builtin_readonly(&mut env, &words(&["readonly", "RO_NUL=a\0b"])).unwrap(),
            1
        );
        assert!(!env.is_readonly("RO_NUL"));
        assert_eq!(
            builtin_readonly(&mut env, &words(&["readonly", "RO_OK=1"])).unwrap(),
            0
        );
        assert!(env.is_readonly("RO_OK"));
    }
}

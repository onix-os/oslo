//! `local` and `readonly`: the two builtins that attach a *scope* or an *attribute* to a name.

use super::options;
use super::quoting::single_quoted;
use crate::env::scope::{Environment, is_valid_identifier};
use crate::error::Result;

const LOCAL_USAGE: &str = "usage: local [-irx] name[=value] ...";
const READONLY_USAGE: &str = "usage: readonly [-p] [name[=value] ...]";

/// Split `name=value` into its halves; a bare name has no value, which is not the same as an
/// empty one (`local x` leaves an existing `x` visible, `local x=` blanks it).
fn split_assignment(arg: &str) -> (&str, Option<&str>) {
    match arg.find('=') {
        Some(idx) => (&arg[..idx], Some(&arg[idx + 1..])),
        None => (arg, None),
    }
}

/// `local [-irx] name[=value] ...`.
///
/// The options are parsed rather than assigned to: `local -r x=1` used to create a variable
/// literally called `-r` and then a second one called `x`.
///
/// `-i` (integer) is accepted and otherwise ignored — rush keeps no attribute table, so there is
/// nothing to record and arithmetic on assignment is not implemented. Rejecting it outright
/// would break scripts for a declaration that is only ever an optimisation hint.
pub fn builtin_local(env: &mut Environment, args: &[String]) -> Result<i32> {
    let opts = match options::parse(args, "irx") {
        Ok(o) => o,
        Err(letter) => return Ok(options::invalid("local", letter, LOCAL_USAGE)),
    };

    let mut status = 0;
    for arg in &args[opts.operands..] {
        let (name, value) = split_assignment(arg);
        if !is_valid_identifier(name) {
            super::not_an_identifier("local", arg);
            status = 1;
            continue;
        }
        let assigned = if opts.has('x') {
            env.set_local_exported_var(name, value.unwrap_or_default())
        } else {
            env.set_local_var(name, value.unwrap_or_default())
        };
        if !assigned {
            status = 1;
            continue;
        }
        // Marked only after the assignment succeeded: a name locked read-only with no value is
        // a name nobody can ever use again.
        if opts.has('r') {
            env.set_readonly(name);
        }
    }

    Ok(status)
}

/// `readonly [-p] [name[=value] ...]`.
///
/// The listing prints values, not just names. `readonly x` on its own told you a name was
/// read-only but not what it was frozen to, which is the one thing you cannot find out any other
/// way once the variable can no longer be assigned.
pub fn builtin_readonly(env: &mut Environment, args: &[String]) -> Result<i32> {
    let opts = match options::parse(args, "p") {
        Ok(o) => o,
        Err(letter) => return Ok(options::invalid("readonly", letter, READONLY_USAGE)),
    };
    let operands = &args[opts.operands..];

    if operands.is_empty() || opts.has('p') {
        print_listing(env);
        return Ok(0);
    }

    let mut status = 0;
    for arg in operands {
        let (name, assigned) = split_assignment(arg);
        if !is_valid_identifier(name) {
            super::not_an_identifier("readonly", arg);
            status = 1;
            continue;
        }
        // A refused assignment must not leave the name read-only: the user would then be unable
        // to set it at all, with nothing to show for it.
        if let Some(value) = assigned
            && !env.set_var(name, value, false)
        {
            status = 1;
            continue;
        }
        env.set_readonly(name);
    }

    Ok(status)
}

/// Print the read-only variables as `readonly name='value'`, sorted.
fn print_listing(env: &Environment) {
    let vars = env.get_all_vars();
    let mut names: Vec<&String> = vars
        .keys()
        .filter(|name| is_valid_identifier(name) && env.is_readonly(name))
        .collect();
    names.sort();
    for name in names {
        println!("readonly {}={}", name, single_quoted(&vars[name]));
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::words;
    use super::{builtin_local, builtin_readonly};
    use crate::env::scope::Environment;

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

    /// `local -r x=1` used to declare two variables, one of them called `-r`.
    #[test]
    fn local_options_are_not_names() {
        let mut env = Environment::new();
        env.push_scope();
        assert_eq!(
            builtin_local(&mut env, &words(&["local", "-r", "LOCAL_RO=1"])).unwrap(),
            0
        );
        assert!(env.get_var("-r").is_none());
        assert!(env.get_var("r").is_none());
        assert_eq!(env.get_var("LOCAL_RO"), Some("1"));
        assert!(env.is_readonly("LOCAL_RO"));
        env.pop_scope();
    }

    #[test]
    fn local_i_is_accepted_without_becoming_a_variable() {
        let mut env = Environment::new();
        env.push_scope();
        assert_eq!(
            builtin_local(&mut env, &words(&["local", "-i", "LOCAL_INT=7"])).unwrap(),
            0
        );
        assert!(env.get_var("-i").is_none());
        assert_eq!(env.get_var("LOCAL_INT"), Some("7"));
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

    /// `readonly -p` used to be an assignment to a variable called `-p`.
    #[test]
    fn readonly_p_lists_instead_of_declaring() {
        let mut env = Environment::new();
        assert_eq!(
            builtin_readonly(&mut env, &words(&["readonly", "-p"])).unwrap(),
            0
        );
        assert!(!env.is_readonly("-p"));
    }
}

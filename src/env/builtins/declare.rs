//! `declare` / `typeset` — declare variables and their attributes.
//!
//! Registered under both names because they are the same builtin; `typeset` is the ksh spelling
//! bash keeps for compatibility.
//!
//! The scoping rule is inherited rather than reimplemented: [`Environment::set_local_var`] writes
//! into the innermost scope frame when there is one and straight to the global table when there
//! is not, which is exactly `declare`'s "local inside a function, global outside" contract.
//! `-g` opts out by writing globally either way.
//!
//! Attributes rush cannot represent are **refused**, not ignored. `declare -i n` in bash makes
//! every later assignment to `n` an arithmetic evaluation, and `declare -a` makes an array
//! (Round 8); accepting either and quietly producing a plain scalar is the "plausible wrong
//! answer with status 0" failure mode this shell is being audited for, so they exit 2 with a
//! diagnostic instead.
//!
//! [`Environment::set_local_var`]: crate::env::Environment::set_local_var

use crate::env::scope::{Environment, is_valid_identifier};
use crate::error::Result;

/// What the option letters asked for.
#[derive(Default)]
struct Attributes {
    export: bool,
    readonly: bool,
    global: bool,
    print: bool,
    functions: bool,
}

/// `declare [-fFgprx] [name[=value] …]`, and `typeset`, its other name.
pub fn builtin_declare(env: &mut Environment, args: &[String]) -> Result<i32> {
    let name = args.first().map(String::as_str).unwrap_or("declare");
    let mut attrs = Attributes::default();

    let mut i = 1;
    while i < args.len() {
        let arg = &args[i];
        if arg == "--" {
            i += 1;
            break;
        }
        if arg.len() < 2 || !(arg.starts_with('-') || arg.starts_with('+')) {
            break;
        }
        for c in arg[1..].chars() {
            match (arg.starts_with('+'), c) {
                (false, 'x') => attrs.export = true,
                (false, 'r') => attrs.readonly = true,
                (false, 'g') => attrs.global = true,
                (false, 'p') => attrs.print = true,
                // `-f` and `-F` are treated alike: bash's `-f` prints each function's body, and
                // rush has no way to render an AST back to source that would not be a guess at
                // what the author wrote. Both therefore report the name only, as `-F` does.
                (false, 'f' | 'F') => attrs.functions = true,
                // Every remaining letter names an attribute this shell has no representation
                // for. Saying so beats declaring a scalar and calling it an array.
                (plus, c) => {
                    let sign = if plus { '+' } else { '-' };
                    eprintln!("rush: {}: {}{}: attribute not supported", name, sign, c);
                    return Ok(2);
                }
            }
        }
        i += 1;
    }

    let operands = &args[i.min(args.len())..];

    if attrs.functions {
        return Ok(print_functions(env, operands));
    }
    if attrs.print || operands.is_empty() {
        return Ok(print_variables(env, operands));
    }

    let mut status = 0;
    for operand in operands {
        if !apply(env, operand, &attrs, name) {
            status = 1;
        }
    }
    Ok(status)
}

/// Declare one `name` or `name=value`; `false` if it was refused.
fn apply(env: &mut Environment, operand: &str, attrs: &Attributes, builtin: &str) -> bool {
    let (name, value) = match operand.split_once('=') {
        Some((name, value)) => (name, Some(value)),
        None => (operand, None),
    };

    if !is_valid_identifier(name) {
        eprintln!("rush: {}: `{}': not a valid identifier", builtin, operand);
        return false;
    }

    if let Some(value) = value {
        // Without `-g` this is `local` inside a function and a plain assignment outside, which is
        // what `set_local_var` already decides based on whether a scope frame exists.
        let assigned = if attrs.global {
            env.set_var(name, value, attrs.export)
        } else {
            env.set_local_var(name, value)
        };
        if !assigned {
            return false;
        }
    }

    if attrs.export && !env.export_var(name) {
        return false;
    }
    if attrs.readonly {
        // Last, so that `declare -r x=1` gets its value in before the variable is frozen.
        env.set_readonly(name);
    }
    true
}

/// `declare -p [name…]` — print declarations in a form the shell could read back.
fn print_variables(env: &mut Environment, names: &[String]) -> i32 {
    let exported = env.get_exported_vars();
    let mut status = 0;

    let render = |env: &Environment, name: &str, value: &str| {
        let mut flags = String::new();
        if env.is_readonly(name) {
            flags.push('r');
        }
        if exported.contains_key(name) {
            flags.push('x');
        }
        let flags = if flags.is_empty() {
            "--".to_string()
        } else {
            format!("-{}", flags)
        };
        println!("declare {} {}=\"{}\"", flags, name, escape(value));
    };

    if names.is_empty() {
        let mut all: Vec<(String, String)> = env.get_all_vars().into_iter().collect();
        all.sort();
        for (name, value) in &all {
            render(env, name, value);
        }
        return 0;
    }

    for name in names {
        // A `name=value` operand is a declaration even under `-p`; only bare names are queries.
        let name = name.split_once('=').map(|(n, _)| n).unwrap_or(name);
        match env.get_var(name).map(str::to_string) {
            Some(value) => render(env, name, &value),
            None => {
                eprintln!("rush: declare: {}: not found", name);
                status = 1;
            }
        }
    }
    status
}

/// `declare -f`/`-F` — report shell functions.
fn print_functions(env: &Environment, names: &[String]) -> i32 {
    let selected: Vec<String> = if names.is_empty() {
        let mut all: Vec<String> = env.get_functions().keys().cloned().collect();
        all.sort();
        all
    } else {
        names.to_vec()
    };

    let mut status = 0;
    for name in selected {
        if env.get_function(&name).is_none() {
            status = 1;
            continue;
        }
        println!("declare -f {}", name);
    }
    status
}

/// Quote a value for `declare -p` output so it could be pasted back into the shell.
fn escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        if matches!(c, '"' | '\\' | '$' | '`') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{builtin_declare, escape};
    use crate::env::Environment;

    fn argv(words: &[&str]) -> Vec<String> {
        words.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_plain_declaration_assigns() {
        let mut env = Environment::new();
        assert_eq!(
            builtin_declare(&mut env, &argv(&["declare", "rush_d1=value"])).unwrap(),
            0
        );
        assert_eq!(env.get_var("rush_d1"), Some("value"));
    }

    #[test]
    fn export_and_readonly_attributes_are_applied() {
        let mut env = Environment::new();
        assert_eq!(
            builtin_declare(&mut env, &argv(&["declare", "-rx", "rush_d2=v"])).unwrap(),
            0
        );
        assert!(env.get_exported_vars().contains_key("rush_d2"));
        assert!(env.is_readonly("rush_d2"));
    }

    /// `-r` is applied after the value, or `declare -r x=1` would freeze `x` before it had one.
    #[test]
    fn a_readonly_declaration_keeps_its_initial_value() {
        let mut env = Environment::new();
        builtin_declare(&mut env, &argv(&["declare", "-r", "rush_d3=kept"])).unwrap();
        assert_eq!(env.get_var("rush_d3"), Some("kept"));
    }

    /// Inside a scope frame — a function call — a declaration is local and is undone on exit.
    #[test]
    fn a_declaration_inside_a_function_is_local() {
        let mut env = Environment::new();
        env.set_var("rush_d4", "outer", false);
        env.push_scope();
        builtin_declare(&mut env, &argv(&["declare", "rush_d4=inner"])).unwrap();
        assert_eq!(env.get_var("rush_d4"), Some("inner"));
        env.pop_scope();
        assert_eq!(env.get_var("rush_d4"), Some("outer"));
    }

    /// `-g` is the opt-out: the assignment outlives the frame it was made in.
    #[test]
    fn a_global_declaration_escapes_the_frame() {
        let mut env = Environment::new();
        env.push_scope();
        builtin_declare(&mut env, &argv(&["declare", "-g", "rush_d5=global"])).unwrap();
        env.pop_scope();
        assert_eq!(env.get_var("rush_d5"), Some("global"));
    }

    /// An attribute with no representation in this shell is refused, not silently downgraded to
    /// a plain scalar.
    #[test]
    fn an_unrepresentable_attribute_is_refused() {
        let mut env = Environment::new();
        assert_eq!(
            builtin_declare(&mut env, &argv(&["declare", "-a", "rush_d6"])).unwrap(),
            2
        );
        assert_eq!(env.get_var("rush_d6"), None);
        assert_eq!(
            builtin_declare(&mut env, &argv(&["declare", "-i", "rush_d7=1"])).unwrap(),
            2
        );
        assert_eq!(env.get_var("rush_d7"), None);
    }

    #[test]
    fn an_invalid_name_is_reported() {
        let mut env = Environment::new();
        assert_eq!(
            builtin_declare(&mut env, &argv(&["declare", "1bad=x"])).unwrap(),
            1
        );
    }

    #[test]
    fn a_missing_name_under_p_is_reported() {
        let mut env = Environment::new();
        assert_eq!(
            builtin_declare(&mut env, &argv(&["declare", "-p", "rush_no_such_var"])).unwrap(),
            1
        );
    }

    #[test]
    fn values_are_quoted_so_they_can_be_read_back() {
        assert_eq!(escape(r#"a"b"#), r#"a\"b"#);
        assert_eq!(escape("a$b`c\\d"), "a\\$b\\`c\\\\d");
        assert_eq!(escape("plain"), "plain");
    }
}

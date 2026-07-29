//! `alias` and `unalias`.

use super::options;
use super::quoting::single_quoted;
use crate::env::scope::Environment;
use crate::error::Result;

const ALIAS_USAGE: &str = "usage: alias [-p] [name[=value] ...]";
const UNALIAS_USAGE: &str = "usage: unalias [-a] name [name ...]";

/// Whether `name` can be used as an alias name.
///
/// Wider than a variable name — `ll`, `..` and `l.` are all ordinary aliases — but not
/// unlimited: a name containing whitespace or shell metacharacters could never be typed as a
/// command word, so accepting one would create an alias that can only ever be listed.
fn is_valid_alias_name(name: &str) -> bool {
    !name.is_empty()
        && !name
            .chars()
            .any(|c| c.is_whitespace() || "'\"`$&;|<>()\\=".contains(c))
}

/// `alias [name[=value] ...]`.
///
/// With no operands this prints the whole table. It used to print nothing at all, which made the
/// aliases the shell seeds itself with invisible — and made `alias | grep` silently useless.
pub fn builtin_alias(env: &mut Environment, args: &[String]) -> Result<i32> {
    let opts = match options::parse(args, "p") {
        Ok(o) => o,
        Err(letter) => return Err(options::invalid("alias", letter, ALIAS_USAGE)),
    };
    let operands = &args[opts.operands..];

    if operands.is_empty() || opts.has('p') {
        let aliases = env.get_aliases();
        let mut names: Vec<&String> = aliases.keys().collect();
        names.sort();
        for name in names {
            println!("alias {}={}", name, single_quoted(&aliases[name]));
        }
        return Ok(0);
    }

    let mut status = 0;
    for arg in operands {
        match arg.find('=') {
            Some(idx) => {
                let name = &arg[..idx];
                if !is_valid_alias_name(name) {
                    eprintln!("rush: alias: `{}': invalid alias name", name);
                    status = 1;
                    continue;
                }
                env.set_alias(name, &arg[idx + 1..]);
            }
            None => match env.get_alias(arg) {
                Some(value) => println!("alias {}={}", arg, single_quoted(value)),
                None => {
                    eprintln!("rush: alias: {}: not found", arg);
                    status = 1;
                }
            },
        }
    }

    Ok(status)
}

/// `unalias [-a] name [name ...]`.
pub fn builtin_unalias(env: &mut Environment, args: &[String]) -> Result<i32> {
    let opts = match options::parse(args, "a") {
        Ok(o) => o,
        Err(letter) => return Err(options::invalid("unalias", letter, UNALIAS_USAGE)),
    };

    if opts.has('a') {
        for name in env.get_aliases().keys().cloned().collect::<Vec<_>>() {
            env.remove_alias(&name);
        }
        return Ok(0);
    }

    let operands = &args[opts.operands..];
    if operands.is_empty() {
        eprintln!("{}", UNALIAS_USAGE);
        return Ok(2);
    }

    let mut status = 0;
    for name in operands {
        // Removing something that was not there is a failure, not a no-op: `unalias ls ||
        // add_default` has to be able to tell the difference.
        if env.get_alias(name).is_none() {
            eprintln!("rush: unalias: {}: not found", name);
            status = 1;
            continue;
        }
        env.remove_alias(name);
    }

    Ok(status)
}

#[cfg(test)]
mod tests {
    use super::super::tests::words;
    use super::{builtin_alias, builtin_unalias, is_valid_alias_name};
    use crate::env::scope::Environment;

    #[test]
    fn unalias_reports_a_name_that_was_not_there() {
        let mut env = Environment::new();
        assert_eq!(
            builtin_unalias(&mut env, &words(&["unalias", "no_such_alias"])).unwrap(),
            1
        );
    }

    /// `-a` used to be taken as the name of an alias to delete, so it removed nothing.
    #[test]
    fn unalias_a_clears_the_table() {
        let mut env = Environment::new();
        env.set_alias("keep", "echo keep");
        assert_eq!(
            builtin_unalias(&mut env, &words(&["unalias", "-a"])).unwrap(),
            0
        );
        assert!(env.get_aliases().is_empty());
    }

    /// `unalias -- -a` operates on an alias genuinely called `-a`.
    #[test]
    fn double_dash_reaches_a_name_that_looks_like_an_option() {
        let mut env = Environment::new();
        env.set_alias("-a", "echo dash-a");
        assert_eq!(
            builtin_unalias(&mut env, &words(&["unalias", "--", "-a"])).unwrap(),
            0
        );
        assert!(env.get_alias("-a").is_none());
    }

    #[test]
    fn an_alias_naming_itself_is_rejected() {
        assert!(is_valid_alias_name("ll"));
        assert!(is_valid_alias_name(".."));
        assert!(!is_valid_alias_name(""));
        assert!(!is_valid_alias_name("a b"));
        assert!(!is_valid_alias_name("a|b"));
    }

    #[test]
    fn listing_a_missing_alias_fails() {
        let mut env = Environment::new();
        assert_eq!(
            builtin_alias(&mut env, &words(&["alias", "no_such_alias"])).unwrap(),
            1
        );
    }
}

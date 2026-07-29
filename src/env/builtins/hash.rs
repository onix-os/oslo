//! `hash` — the shell's remembered command locations.
//!
//! The cache is a real one rather than a stub, because the observable behaviour scripts depend on
//! is `hash -r` *forgetting*: install a new binary earlier on `PATH` and the shell must find it.
//! A shell that never caches passes that test by accident, but then `hash foo` cannot report
//! "not found" for a name that does not exist, which is the other half of the contract.
//!
//! The table lives in a thread-local rather than in [`Environment`], so it is per-process: a
//! forked subshell inherits a copy and its own `hash -r` cannot reach back into the parent, which
//! is what bash does too.

use crate::env::builtins::spawn::resolve_program;
use crate::env::scope::Environment;
use crate::error::Result;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::PathBuf;

thread_local! {
    /// Command word to (resolved path, number of times the shell has looked it up).
    static TABLE: RefCell<BTreeMap<String, (PathBuf, u32)>> =
        const { RefCell::new(BTreeMap::new()) };
}

/// `hash [-r] [name…]`.
pub fn builtin_hash(_env: &mut Environment, args: &[String]) -> Result<i32> {
    let mut i = 1;
    while i < args.len() {
        let arg = &args[i];
        if arg == "--" {
            i += 1;
            break;
        }
        if arg.len() < 2 || !arg.starts_with('-') {
            break;
        }
        for c in arg[1..].chars() {
            match c {
                'r' => TABLE.with(|t| t.borrow_mut().clear()),
                other => {
                    eprintln!("rush: hash: -{}: invalid option", other);
                    eprintln!("hash: usage: hash [-r] [name ...]");
                    return Ok(2);
                }
            }
        }
        i += 1;
    }

    let names = &args[i.min(args.len())..];
    if names.is_empty() {
        print_table();
        return Ok(0);
    }

    let mut status = 0;
    for name in names {
        match resolve_program(name) {
            Some(path) => TABLE.with(|t| {
                t.borrow_mut().insert(name.clone(), (path, 0));
            }),
            None => {
                eprintln!("rush: hash: {}: not found", name);
                status = 1;
            }
        }
    }
    Ok(status)
}

/// Record that `name` resolved to `path`, so `hash` can report it.
///
/// Exposed for the command-resolution path in [`crate::exec::simple`], which is where lookups
/// actually happen; until that calls it the table only holds what `hash name` put there.
pub fn remember(name: &str, path: PathBuf) {
    TABLE.with(|t| {
        let mut table = t.borrow_mut();
        let entry = table.entry(name.to_string()).or_insert((path, 0));
        entry.1 += 1;
    });
}

/// Look up a remembered location, counting the hit.
pub fn recall(name: &str) -> Option<PathBuf> {
    TABLE.with(|t| {
        let mut table = t.borrow_mut();
        let entry = table.get_mut(name)?;
        entry.1 += 1;
        Some(entry.0.clone())
    })
}

fn print_table() {
    TABLE.with(|t| {
        let table = t.borrow();
        if table.is_empty() {
            // Not an error: bash says so on stdout and exits 0.
            println!("hash: hash table empty");
            return;
        }
        println!("hits\tcommand");
        for (path, hits) in table.values() {
            println!("{:>4}\t{}", hits, path.display());
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{builtin_hash, recall, remember};
    use crate::env::Environment;
    use std::path::PathBuf;

    fn argv(words: &[&str]) -> Vec<String> {
        words.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_known_command_is_remembered_and_a_missing_one_reported() {
        let mut env = Environment::new();
        assert_eq!(builtin_hash(&mut env, &argv(&["hash", "sh"])).unwrap(), 0);
        assert_eq!(
            builtin_hash(&mut env, &argv(&["hash", "no-such-command-xyz"])).unwrap(),
            1
        );
    }

    /// `hash -r` exists so that a newly installed binary is found; a table that survives it would
    /// keep handing back the old path forever.
    #[test]
    fn resetting_forgets_everything() {
        let mut env = Environment::new();
        remember("rush-hash-test", PathBuf::from("/nowhere/rush-hash-test"));
        assert!(recall("rush-hash-test").is_some());
        assert_eq!(builtin_hash(&mut env, &argv(&["hash", "-r"])).unwrap(), 0);
        assert!(recall("rush-hash-test").is_none());
    }

    #[test]
    fn an_unknown_option_is_a_usage_error() {
        let mut env = Environment::new();
        assert_eq!(builtin_hash(&mut env, &argv(&["hash", "-Z"])).unwrap(), 2);
    }
}

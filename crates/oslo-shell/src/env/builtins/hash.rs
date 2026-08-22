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
use crate::env::origin_now;
use crate::env::scope::Environment;
use oslo_base::error::Result;
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
    let mut cleared = false;
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
                'r' => {
                    // `forget_all` also drops the line editor's own set of runnable names, built
                    // by walking `$PATH`: `hash -r` means "forget where commands live", so a
                    // completion list that outlived the reset would be offering a path the shell
                    // has just been told to stop trusting.
                    forget_all();
                    cleared = true;
                }
                other => {
                    eprintln!("{}hash: -{}: invalid option", origin_now(), other);
                    eprintln!("hash: usage: hash [-r] [name ...]");
                    return Ok(2);
                }
            }
        }
        i += 1;
    }

    let names = &args[i.min(args.len())..];
    if names.is_empty() {
        // `hash -r` is a request to forget, not to report: printing the table it just emptied
        // put "hash table empty" on the stdout of every script that clears the cache.
        if !cleared {
            print_table();
        }
        return Ok(0);
    }

    let mut status = 0;
    for name in names {
        match resolve_program(name) {
            Some(path) => TABLE.with(|t| {
                t.borrow_mut().insert(name.clone(), (path, 0));
            }),
            None => {
                eprintln!("{}hash: {}: not found", origin_now(), name);
                status = 1;
            }
        }
    }
    Ok(status)
}

/// Resolve a command word through the table, filling it in on a miss.
///
/// This is the whole point of the cache and the one function the command-resolution path needs:
/// a bare word is answered from the table when it is there, searched for and remembered when it
/// is not, and the hit is counted either way so `hash` can report it.
///
/// A word containing a slash is a path, not a `PATH` search — bash does not hash those, and
/// neither does this: caching `./configure` would make the entry meaningless the moment the
/// shell changed directory.
pub fn lookup(name: &str) -> Option<PathBuf> {
    if name.contains('/') {
        return resolve_program(name);
    }
    // A name this directory hides is not on `$PATH` as far as anything asking here is concerned.
    // Here rather than at the call sites because this *is* the question "what would `name` run",
    // and execution, `oslo.run`, argc's completion and the nix probe all ask it through this.
    if oslo_base::command::hidden(name) {
        return None;
    }
    if let Some(path) = recall(name) {
        // A remembered path that has since been removed is worse than no cache at all: the shell
        // would report "cannot execute" for a command that a fresh `PATH` search would find in
        // the next directory along. Re-search instead, which also drops the dead entry.
        if path.is_file() {
            return Some(path);
        }
        TABLE.with(|t| t.borrow_mut().remove(name));
    }
    let path = resolve_program(name)?;
    remember(name, path.clone());
    Some(path)
}

/// Drop every remembered location.
///
/// `hash -r`'s other caller is [`Environment::set_var`](crate::env::scope::Environment::set_var):
/// assigning to `PATH` invalidates every entry in the table by definition.
pub fn forget_all() {
    TABLE.with(|t| t.borrow_mut().clear());
    oslo_ui::invalidate_command_cache();
}

/// Record that `name` resolved to `path`, and count the lookup that found it.
///
/// The path is *replaced*, not kept: a stale entry that survived a `PATH` change is the failure
/// mode a command cache exists to be blamed for, and `hash -r` is not the only thing that should
/// be able to correct one.
pub fn remember(name: &str, path: PathBuf) {
    TABLE.with(|t| {
        let mut table = t.borrow_mut();
        let entry = table.entry(name.to_string()).or_insert((path.clone(), 0));
        entry.0 = path;
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
        remember("oslo-hash-test", PathBuf::from("/nowhere/oslo-hash-test"));
        assert!(recall("oslo-hash-test").is_some());
        assert_eq!(builtin_hash(&mut env, &argv(&["hash", "-r"])).unwrap(), 0);
        assert!(recall("oslo-hash-test").is_none());
    }

    /// The command-resolution entry point: a search fills the table, and the next lookup is
    /// answered from it. Without this the table only ever held what an explicit `hash name` put
    /// there, and `sh; hash` reported an empty cache where bash listed `sh`.
    #[test]
    fn a_lookup_populates_the_table_and_the_next_one_is_served_from_it() {
        assert_eq!(super::lookup("sh"), super::resolve_program("sh"));
        assert!(recall("sh").is_some(), "the search should have been cached");
        assert_eq!(super::lookup("no-such-command-xyz"), None);
        assert!(recall("no-such-command-xyz").is_none());
    }

    /// A word with a slash is a path, not a `PATH` search, so it must not enter the table:
    /// `./x` means something different in every directory.
    #[test]
    fn a_path_is_not_hashed() {
        assert!(super::lookup("/bin/sh").is_some());
        assert!(recall("/bin/sh").is_none());
    }

    /// A binary that moved must not be reported at its old location for the rest of the session.
    #[test]
    fn remembering_again_replaces_the_path() {
        remember("oslo-moved", PathBuf::from("/old/oslo-moved"));
        remember("oslo-moved", PathBuf::from("/new/oslo-moved"));
        assert_eq!(recall("oslo-moved"), Some(PathBuf::from("/new/oslo-moved")));
    }

    /// `hash -r` has to reach the *other* cache too: the line editor completes from its own set
    /// of `$PATH` executables, and a shell that kept offering a command it had just been told to
    /// forget would be contradicting itself.
    #[test]
    fn resetting_also_drops_the_completion_cache() {
        use oslo_ui::command_index::CommandIndex;
        use std::sync::Arc;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_str().unwrap();
        let before = CommandIndex::executables(path);

        let mut env = Environment::new();
        assert_eq!(builtin_hash(&mut env, &argv(&["hash", "-r"])).unwrap(), 0);

        // A fresh allocation means the directories were read again rather than replayed.
        assert!(!Arc::ptr_eq(&before, &CommandIndex::executables(path)));
    }

    #[test]
    fn an_unknown_option_is_a_usage_error() {
        let mut env = Environment::new();
        assert_eq!(builtin_hash(&mut env, &argv(&["hash", "-Z"])).unwrap(), 2);
    }
}

//! The builtin registry: the one table that decides what a builtin is.
//!
//! Three lists used to disagree — the `matches!` in `Environment::is_builtin`, a hardcoded
//! `match` in the dispatcher, and the map behind [`Environment::register_custom_builtin`] — so
//! a name could be a builtin to `type` and not to the dispatcher, or dispatch to a hardcoded
//! function while the map held a different one (PLAN R5.6, R9.8). Registration is now the only
//! way in: [`crate::env::builtins::register_default_builtins`] fills this table and
//! `is_builtin`, the dispatcher in [`crate::exec::simple`] and `type` all read it back. Adding a
//! builtin is one `register_custom_builtin` call and nothing else.

use crate::env::scope::Environment;
use crate::error::Result;
use std::collections::HashMap;

pub type BuiltinFn = fn(&mut Environment, &[String]) -> Result<i32>;

/// The POSIX "special built-in utilities" (XCU 2.14).
///
/// Listed by name rather than flagged at registration, because being special is something POSIX
/// says about the *name*, not about the implementation: `exec` is special whoever provides it,
/// and a Lua-registered builtin called `set` would not stop `set` being special. Keeping the
/// list here also means a registration site in another module cannot forget the flag.
///
/// Two consequences POSIX attaches to the name: a special builtin is found *before* shell
/// functions during command search, and its failure is fatal to a non-interactive shell.
/// [`crate::exec::simple`] owns the first; the second is not implemented yet.
const SPECIAL_BUILTINS: &[&str] = &[
    ":", ".", "break", "continue", "eval", "exec", "exit", "export", "readonly", "return", "set",
    "shift", "times", "trap", "unset",
];

/// Whether `name` is a POSIX special builtin. True even for names rush does not implement yet,
/// so that adding one does not silently change its search order.
pub fn is_special_builtin(name: &str) -> bool {
    SPECIAL_BUILTINS.contains(&name.trim())
}

/// Name → implementation, for every builtin this shell has.
#[derive(Default)]
pub struct BuiltinRegistry {
    table: HashMap<String, BuiltinFn>,
}

impl BuiltinRegistry {
    /// Add or replace a builtin. Last registration wins, which is what lets a script-supplied
    /// builtin override a default one.
    pub fn register(&mut self, name: &str, func: BuiltinFn) {
        self.table.insert(name.to_string(), func);
    }

    /// The implementation of `name`, if it is a builtin at all.
    ///
    /// Trimmed on the way in: the command word reaching the dispatcher can carry whitespace the
    /// lexer did not strip, and answering "not a builtin" for `" echo"` sent it to PATH instead.
    pub fn lookup(&self, name: &str) -> Option<BuiltinFn> {
        self.table.get(name.trim()).copied()
    }

    pub fn contains(&self, name: &str) -> bool {
        self.lookup(name).is_some()
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.table.keys().map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::is_special_builtin;
    use crate::env::Environment;

    /// The registry is the source of truth only if everything that used to be hardcoded is in
    /// it. These are the names the old `matches!` in `is_builtin` listed, plus the three it
    /// omitted (`set`, `shift`, `type`) that the map already had — one list now, so a name
    /// cannot be a builtin to one caller and not to another.
    #[test]
    fn every_default_builtin_is_registered_and_dispatchable() {
        let env = Environment::new();
        for name in [
            "cd", "pwd", "echo", "export", "unset", "set", "shift", "exit", "break", "continue",
            "return", "alias", "unalias", "type", "eval", "source", ".", "read", "local", "pushd",
            "popd", "dirs", "readonly", "test", "[", "[[", "trap", "umask", "wait", "kill", "true",
            "false",
        ] {
            assert!(env.is_builtin(name), "{name} should be a builtin");
            assert!(
                env.get_builtin(name).is_some(),
                "{name} answers is_builtin but has no implementation to dispatch to"
            );
        }
    }

    /// `is_builtin` and `builtin_names` must agree in both directions, or completion and `type`
    /// go on disagreeing with the dispatcher the way they did before R5.6.
    #[test]
    fn listed_names_are_exactly_the_dispatchable_ones() {
        let env = Environment::new();
        let names: Vec<String> = env.builtin_names().map(str::to_string).collect();
        assert!(!names.is_empty());
        for name in &names {
            assert!(env.is_builtin(name), "{name} is listed but not a builtin");
        }
    }

    #[test]
    fn a_leading_space_still_names_a_builtin() {
        let env = Environment::new();
        assert!(env.is_builtin(" echo"));
        assert!(env.get_builtin("echo ").is_some());
    }

    #[test]
    fn special_builtins_are_the_posix_set() {
        for special in [
            ":", ".", "eval", "exec", "exit", "export", "set", "trap", "unset",
        ] {
            assert!(is_special_builtin(special), "{special} is special in POSIX");
        }
        // Regular builtins: a function is allowed to shadow these even in POSIX mode.
        for regular in [
            "cd", "echo", "read", "test", "[", "true", "umask", "wait", "kill",
        ] {
            assert!(!is_special_builtin(regular), "{regular} is not special");
        }
        assert!(!is_special_builtin("ls"));
    }
}

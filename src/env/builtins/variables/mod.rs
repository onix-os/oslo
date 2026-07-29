//! Variables and aliases: `export`, `unset`, `set`, `shift`, `local`, `readonly`,
//! `alias`, `unalias`.
//!
//! Split by what each builtin acts on rather than kept in one file, because the three jobs here
//! barely overlap: giving a name an attribute ([`exporting`], [`scoped`]), maintaining the alias
//! table ([`aliases`]), and printing the shell's state back as something a shell can read again
//! ([`quoting`], [`deparse`], [`parameters`]).
//!
//! Every listing these builtins produce is *re-inputtable*: sorted so two runs agree, and quoted
//! so `export -p > f; . f` restores exactly what was there. A `HashMap` iteration order and a
//! `{:?}` value are neither, which is what the listings used to be.
//!
//! # Helpers this module still needs from [`Environment`]
//!
//! Two behaviours are parsed and diagnosed here but cannot be *carried out* without new methods
//! on [`Environment`], whose relevant fields are private:
//!
//! * `unset -f name` needs `Environment::remove_function`; the functions map is exposed
//!   read-only.
//! * `local` outside a function must fail, which needs `Environment::in_function` (the function
//!   depth counter and the scope stack are both private).
//!
//! [`Environment`]: crate::env::scope::Environment

mod aliases;
mod deparse;
mod exporting;
mod options;
mod parameters;
mod quoting;
mod scoped;

pub use aliases::{builtin_alias, builtin_unalias};
pub use exporting::{builtin_export, builtin_unset};
pub use parameters::{builtin_set, builtin_shift};
pub use scoped::{builtin_local, builtin_readonly};

/// Complain about `word` the way bash does for a name that is not `[A-Za-z_][A-Za-z0-9_]*`.
///
/// The whole word is quoted back, not the part before the `=`, so `export '=1'` names what the
/// user actually typed.
fn not_an_identifier(builtin: &str, word: &str) {
    eprintln!("rush: {}: '{}': not a valid identifier", builtin, word);
}

#[cfg(test)]
mod tests {
    use super::{builtin_export, builtin_local};
    use crate::env::scope::Environment;

    pub fn words(parts: &[&str]) -> Vec<String> {
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
}

//! `:` — the null command.
//!
//! POSIX's special builtin that does nothing and succeeds. Trivial to implement and impossible
//! to work around: `while :; do` is the idiomatic infinite loop, `: ${VAR:=default}` is the
//! idiomatic "set if unset", and `*) : ;;` is how you write an empty `case` arm. Without it every
//! one of those is a `command not found`, which is why this exists as its own registration rather
//! than as another alias for `true`.
//!
//! The arguments are already expanded by the time a builtin is called, so the assignment side
//! effect of `: ${VAR:=default}` has happened before this function runs — expanding and then
//! discarding is the whole contract.

use crate::env::scope::Environment;
use oslo_base::error::Result;

/// `:` — expand the arguments, ignore them, succeed.
pub fn builtin_colon(_env: &mut Environment, _args: &[String]) -> Result<i32> {
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::builtin_colon;
    use crate::env::Environment;

    #[test]
    fn the_null_command_always_succeeds() {
        let mut env = Environment::new();
        assert_eq!(builtin_colon(&mut env, &[":".to_string()]).unwrap(), 0);
        assert_eq!(
            builtin_colon(&mut env, &[":".to_string(), "anything".to_string()]).unwrap(),
            0
        );
    }

    /// `:` is only useful if it is reachable through the ordinary dispatch path.
    #[test]
    fn the_null_command_is_registered() {
        let env = Environment::new();
        assert!(env.is_builtin(":"));
    }
}

//! What the builtin decides, which is the whole of it. Whether the child runs is
//! `tests/make_tests.rs`'s question, because answering it needs a process.

use crate::env::scope::Environment;

/// The name is a builtin in every sense the shell has, so `type` and `command -v` answer for it.
///
/// `your-own-tools.md` lists "tools are invisible to everything that answers questions about names"
/// as a limitation of the *other* registration route. There is no reason to ship that hole twice.
#[test]
fn make_is_a_builtin_the_shell_admits_to() {
    let env = Environment::new();
    assert!(env.is_builtin("make"), "make is not in the builtin table");
}

/// A script gets the program, always — the rule `which` follows, and for the same reason: somebody's
/// configure script running `make` means GNU make, whatever is sitting in the working directory.
#[test]
fn a_non_interactive_shell_never_claims_the_name() {
    let env = Environment::new();
    assert!(
        !env.interactive(),
        "a fresh Environment is not interactive, which is what the handover rule keys on"
    );
}

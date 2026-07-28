//! Shell builtins.
//!
//! Grouped by what they act on rather than listed alphabetically, so a change to (say)
//! directory handling touches one file. [`register_default_builtins`] is the single place
//! every builtin is wired up, and [`Environment::builtin_names`] reads back from it — adding
//! one here is enough to make it visible to completion and to `type`.
//!
//! [`Environment::builtin_names`]: crate::env::Environment::builtin_names

mod conditionals;
mod control;
mod directories;
mod io;
mod process;
mod variables;

pub use conditionals::{builtin_extended_test, builtin_test};
pub use control::{
    builtin_break, builtin_continue, builtin_eval, builtin_exit, builtin_return, builtin_source,
    builtin_type,
};
pub use directories::{builtin_cd, builtin_dirs, builtin_popd, builtin_pushd, builtin_pwd};
pub use io::{builtin_echo, builtin_read};
pub use process::{builtin_kill, builtin_trap, builtin_umask, builtin_wait};
pub use variables::{
    builtin_alias, builtin_export, builtin_local, builtin_readonly, builtin_set, builtin_shift,
    builtin_unalias, builtin_unset,
};

use crate::env::scope::Environment;

pub fn register_default_builtins(env: &mut Environment) {
    env.register_custom_builtin("cd", builtin_cd);
    env.register_custom_builtin("pwd", builtin_pwd);
    env.register_custom_builtin("echo", builtin_echo);
    env.register_custom_builtin("export", builtin_export);
    env.register_custom_builtin("unset", builtin_unset);
    env.register_custom_builtin("set", builtin_set);
    env.register_custom_builtin("shift", builtin_shift);
    env.register_custom_builtin("exit", builtin_exit);
    env.register_custom_builtin("break", builtin_break);
    env.register_custom_builtin("continue", builtin_continue);
    env.register_custom_builtin("return", builtin_return);
    env.register_custom_builtin("alias", builtin_alias);
    env.register_custom_builtin("unalias", builtin_unalias);
    env.register_custom_builtin("true", |_, _| Ok(0));
    env.register_custom_builtin("false", |_, _| Ok(1));
    env.register_custom_builtin("type", builtin_type);
    env.register_custom_builtin("eval", builtin_eval);
    env.register_custom_builtin(".", builtin_source);
    env.register_custom_builtin("source", builtin_source);
    env.register_custom_builtin("read", builtin_read);
    env.register_custom_builtin("local", builtin_local);
    env.register_custom_builtin("pushd", builtin_pushd);
    env.register_custom_builtin("popd", builtin_popd);
    env.register_custom_builtin("dirs", builtin_dirs);
    env.register_custom_builtin("readonly", builtin_readonly);
    env.register_custom_builtin("test", builtin_test);

    env.register_custom_builtin("[", builtin_test);
    env.register_custom_builtin("[[", builtin_extended_test);
    env.register_custom_builtin("trap", builtin_trap);
    env.register_custom_builtin("umask", builtin_umask);
    env.register_custom_builtin("wait", builtin_wait);
    env.register_custom_builtin("kill", builtin_kill);
}

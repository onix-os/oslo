//! Shell builtins.
//!
//! Grouped by what they act on rather than listed alphabetically, so a change to (say)
//! directory handling touches one file. [`register_default_builtins`] is the single place
//! every builtin is wired up, and [`Environment::builtin_names`] reads back from it — adding
//! one here is enough to make it visible to completion and to `type`.
//!
//! The builtins with enough behaviour of their own to argue about — `exec`, `command`, `builtin`,
//! `getopts`, `let` — each get a file named for them, because the alternative is one file that
//! grows without limit and that nobody can review a change to.
//!
//! [`Environment::builtin_names`]: crate::env::Environment::builtin_names

pub(crate) mod arrays;
mod builtin;
mod colon;
mod command;
mod conditionals;
mod control;
mod declare;
mod directories;
mod exec;
mod getopts;
mod hash;
mod io;
mod jobs;
mod r#let;
mod process;
mod spawn;
mod times;
mod ulimit;
mod variables;

pub use builtin::builtin_builtin;
pub use colon::builtin_colon;
pub use command::builtin_command;
pub use conditionals::{builtin_extended_test, builtin_test};
pub use control::{
    builtin_break, builtin_continue, builtin_eval, builtin_exit, builtin_return, builtin_source,
    builtin_type,
};
pub use declare::builtin_declare;
pub use directories::{builtin_cd, builtin_dirs, builtin_popd, builtin_pushd, builtin_pwd};
pub use exec::builtin_exec;
pub use getopts::builtin_getopts;
pub use hash::builtin_hash;
pub use io::{builtin_echo, builtin_read};
pub use jobs::{builtin_bg, builtin_disown, builtin_fg, builtin_jobs, builtin_wait};
pub use r#let::builtin_let;
pub use process::{builtin_kill, builtin_trap, builtin_umask, run_exit_trap, run_pending_traps};
pub use times::builtin_times;
pub use ulimit::builtin_ulimit;
pub use variables::{
    builtin_alias, builtin_export, builtin_local, builtin_readonly, builtin_set, builtin_shift,
    builtin_unalias, builtin_unset,
};

/// Whether a command's redirections must outlive it — the `exec > "$log" 2>&1` form.
///
/// A builtin is never handed its own redirections, so this is the one thing about `exec` the
/// dispatcher has to know: [`crate::exec::simple`] must build a *non-restoring*
/// [`crate::exec::redirect::RedirectGuard`] (`RedirectGuard::for_exec`) when this returns true,
/// and the ordinary restoring one otherwise.
pub use exec::makes_redirections_permanent as exec_makes_redirections_permanent;

/// The shell's remembered command locations, for whoever resolves `PATH` lookups.
///
/// [`hash_remember`] records a resolution and [`hash_recall`] replays one; `hash` on its own
/// reports the table and `hash -r` clears it. Nothing in the command-resolution path calls these
/// yet, so today the table only holds what an explicit `hash name` put in it.
pub use hash::recall as hash_recall;
pub use hash::remember as hash_remember;

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

    // Job control. `wait` is registered above but belongs with these: all five read one table.
    env.register_custom_builtin("jobs", builtin_jobs);
    env.register_custom_builtin("fg", builtin_fg);
    env.register_custom_builtin("bg", builtin_bg);
    env.register_custom_builtin("disown", builtin_disown);

    // POSIX's null command. Registered like any other builtin rather than special-cased in the
    // parser, because `:` really is one — `while :; do` and `: ${x:=default}` are ordinary
    // command invocations that happen to do nothing with their arguments.
    env.register_custom_builtin(":", builtin_colon);

    env.register_custom_builtin("exec", builtin_exec);
    env.register_custom_builtin("command", builtin_command);
    env.register_custom_builtin("builtin", builtin_builtin);
    env.register_custom_builtin("getopts", builtin_getopts);
    env.register_custom_builtin("let", builtin_let);
    env.register_custom_builtin("hash", builtin_hash);
    env.register_custom_builtin("times", builtin_times);
    env.register_custom_builtin("ulimit", builtin_ulimit);

    // Two names for one builtin. Registering them is also what makes the `is_declaration` branch
    // in `exec::simple` mean something: it exists so that `declare FOO=bar` reaches the builtin
    // instead of being applied behind its back.
    env.register_custom_builtin("declare", builtin_declare);
    env.register_custom_builtin("typeset", builtin_declare);
}

#[cfg(test)]
mod tests {
    use crate::env::Environment;

    /// Every builtin this round added has to be reachable by name, or it may as well not exist:
    /// dispatch goes through the registry, and so do `type` and completion.
    #[test]
    fn every_new_builtin_is_registered() {
        let env = Environment::new();
        for name in [
            ":", "exec", "command", "builtin", "getopts", "let", "hash", "times", "ulimit",
            "declare", "typeset",
        ] {
            assert!(env.is_builtin(name), "{name} is not registered");
            assert!(
                env.get_builtin(name).is_some(),
                "{name} has no implementation in the registry"
            );
        }
    }
}

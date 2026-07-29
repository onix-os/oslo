//! Forked-child state (R4.1) and the collected pipeline statuses (R4.10).
//!
//! The differential corpus covers what a *script* can observe about a subshell. Two things it
//! cannot observe yet live here instead: the per-stage status vector, which has no shell-visible
//! surface until `PIPESTATUS` (Round 8) and `pipefail` (Round 6) read it, and the fact that a
//! subshell marks itself as one rather than rebuilding a shell from scratch.
//!
//! # Why this file is split in two
//!
//! libtest runs `#[test]` functions as threads of one process, and `environ` belongs to the
//! process. `Environment::set_var` with the export flag set reaches `unsafe { env::set_var }`
//! (`src/env/scope.rs`), whose safety argument is "rush never spawns a thread" — true of the
//! shell, false of this harness, where a sibling test can be inside `Environment::new()`'s
//! `env::vars()` walk reading the pointer array `set_var` is reallocating. That is undefined
//! behaviour rather than flakiness, so it cannot be left to chance.
//!
//! `environ` is not the only process-global thing at stake. A pipeline stage and a command
//! substitution both **fork**, and `fork(2)` in a multi-threaded process gives the child only the
//! calling thread — every lock another libtest worker happened to be holding (the allocator arena,
//! stdio) is inherited locked and can never be released. The child then blocks on that futex
//! forever and the whole test binary hangs, reproducibly at about one run in thirty under
//! `--test-threads=16`. rush itself is single threaded, so this is a property of the harness, not
//! of the shell; the answer is the same as for `environ` — do not do it in process.
//!
//! * [`spawned`] — anything that would write `environ`, the cwd or the umask, **or fork**. These
//!   run the real binary through `tests/common`, so each gets its own process and needs no lock.
//! * [`in_process`] — build an AST, evaluate it, inspect `Environment`. Cheap, and under one
//!   rule: **nothing here may run `export`, `cd` or `umask`, or start a pipeline, a subshell or a
//!   command substitution — in a script or through the API.**

mod common;

/// Tests that would mutate process-global state, each in its own process.
mod spawned {
    use crate::common::run;

    /// R4.1: a subshell keeps every variable it inherited *with its export flag*. Rebuilding the
    /// environment and re-exporting each variable is what leaked private data into every child.
    ///
    /// This ran in process until Round 11, where `export shown=public` reached
    /// `unsafe { env::set_var }` on a libtest worker while six siblings called
    /// `Environment::new()`. It also asserted against `get_exported_vars()` — rush's own
    /// `HashMap`, which would have looked identical had the `environ` write been dropped. Asking
    /// `env(1)` from inside the subshell tests the thing that actually matters, and the prefix
    /// keeps the names out of any real environment.
    #[test]
    fn a_subshell_does_not_export_private_variables() {
        let r = run("RUSH_T_SECRET=classified; export RUSH_T_SHOWN=public; \
             ( echo \"sub=[$RUSH_T_SECRET]\"; env | grep -E '^RUSH_T_' | sort )");
        assert_eq!(r.status, 0, "stderr: {}", r.stderr);
        assert_eq!(
            r.lines(),
            [
                // The subshell still sees the private value...
                "sub=[classified]",
                // ...and its children still do not.
                "RUSH_T_SHOWN=public",
            ],
            "stdout: {:?} stderr: {}",
            r.stdout,
            r.stderr
        );
    }

    /// R4.10: a pipeline that fails in the middle used to leave no trace of it anywhere.
    ///
    /// `PIPESTATUS` (R8) is the shell-visible surface the original in-process version predated,
    /// so the vector can now be asserted from a script — which is also the only safe way to
    /// assert it, since every stage here is a `fork` (see the module docs).
    #[test]
    fn every_pipeline_stage_status_is_recorded() {
        assert_eq!(
            run("false | true; echo \"${PIPESTATUS[*]}\"\n\
                 true | false; echo \"${PIPESTATUS[*]}\"\n\
                 sh -c 'exit 3' | sh -c 'exit 4' | true; echo \"${PIPESTATUS[*]}\"")
            .lines(),
            ["1 0", "0 1", "3 4 0"]
        );
    }

    /// A one-command pipeline still has a stage vector, as bash's `PIPESTATUS` does.
    #[test]
    fn a_single_command_records_one_status() {
        assert_eq!(
            run("true; echo \"${PIPESTATUS[*]}\"\nsh -c 'exit 7'; echo \"${PIPESTATUS[*]}\"")
                .lines(),
            ["0", "7"]
        );
    }

    /// `!` inverts what the pipeline *reports*; the stages themselves still failed.
    #[test]
    fn negation_does_not_rewrite_the_stage_statuses() {
        assert_eq!(
            run("! false | false; echo \"$? ${PIPESTATUS[*]}\"").lines(),
            ["0 1 1"]
        );
    }

    /// The status a command substitution exited with survives the fork it ran in, and is *taken*
    /// rather than merely read: the next assignment must not inherit a stale number.
    #[test]
    fn a_command_substitutions_status_is_kept() {
        assert_eq!(
            run("x=$(exit 5); echo \"$? [$x]\"\ny=plain; echo \"$?\"").lines(),
            ["5 []", "0"]
        );
    }
}

/// Tests that only read and write rush's own state.
mod in_process {
    use rush::env::Environment;
    use rush::exec::eval_command_list;
    use rush::parser::parse_bash_script;

    fn run(env: &mut Environment, script: &str) -> i32 {
        let ast = parse_bash_script(script).expect("parse");
        eval_command_list(env, &ast).expect("execute")
    }

    /// R4.1: the parent shell is never a subshell, and marking one keeps `$$` intact.
    #[test]
    fn entering_a_subshell_keeps_the_invoking_shells_pid() {
        let mut env = Environment::new();
        assert!(!env.in_subshell());

        let dollar_dollar = env.get_param("$");
        env.enter_subshell();

        // Not a real fork, so the pid is unchanged and `in_subshell` cannot flip here; what
        // matters is that `$$` still reports the invoking shell, which is what POSIX and bash
        // require of a subshell. `current_pid` is what job control and `$BASHPID` would use
        // instead.
        assert_eq!(env.get_param("$"), dollar_dollar);
        assert_eq!(env.current_pid(), std::process::id());
    }

    /// R4.1: traps are reset to their default action in a subshell, so an inherited `EXIT`
    /// handler does not fire once per forked child.
    #[test]
    fn a_subshell_starts_with_no_traps() {
        let mut env = Environment::new();
        env.set_trap("EXIT", "echo bye");
        assert_eq!(env.get_trap("EXIT"), Some("echo bye"));

        env.enter_subshell();
        assert_eq!(env.get_trap("EXIT"), None);
        assert!(env.get_traps().is_empty());
    }

    /// R4.1's other half, kept in process because it is about rush's own bookkeeping: an
    /// unexported variable survives `enter_subshell` without acquiring the export flag.
    ///
    /// The `environ`-visible half of this claim is
    /// [`super::spawned::a_subshell_does_not_export_private_variables`]; neither assignment here
    /// is exported, so nothing in this test reaches `environ`.
    #[test]
    fn entering_a_subshell_does_not_set_the_export_flag() {
        let mut env = Environment::new();
        run(&mut env, "secret=classified");
        env.enter_subshell();

        assert!(
            !env.get_exported_vars().contains_key("secret"),
            "private variable exported"
        );
        assert_eq!(env.get_var("secret"), Some("classified"));
    }
}

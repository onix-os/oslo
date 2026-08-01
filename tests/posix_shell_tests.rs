//! Core POSIX shell behaviour.
//!
//! # Why this file is split in two
//!
//! libtest runs `#[test]` functions as threads of a single process. `environ`, the working
//! directory and the umask belong to that process, not to the thread, so a test that changes one
//! of them changes it for every test running at that instant. For the umask and the cwd that is
//! merely wrong answers; for `environ` it is undefined behaviour — `Environment::set_var` reaches
//! `unsafe { std::env::set_var }` (`src/env/scope.rs`), whose safety argument is "oslo never
//! spawns a thread". That argument holds for the shell and is false for its test harness, where
//! another worker thread can be inside `Environment::new()`'s `env::vars()` walk reading the
//! pointer array `set_var` is reallocating.
//!
//! This file used to run all of it as one thread pool with a `TEST_DIR_MUTEX` that three of
//! nineteen tests bothered to take. It passed because the assertions were loose enough not to
//! notice, not because the races were absent. Round 0 also saw an intermittent deadlock here: a
//! `fork()` from a multi-threaded parent gives the child a copy of a malloc lock held by a thread
//! that does not exist in it, and both sides then sit in `futex_do_wait`.
//!
//! So:
//!
//! * **[`process_global`]** — anything that touches `environ`, the cwd or the umask. These spawn
//!   the real binary through `tests/common`. Each gets its own process, hence its own copy of all
//!   three, and needs no lock at all. They also assert more than the in-process versions could:
//!   `export` is checked by looking at a child's environment rather than at a `HashMap`.
//! * **[`in_process`]** — parse, evaluate, inspect `Environment`. These are cheap and stay in
//!   process, under one rule: **nothing here may write to `environ`, the cwd or the umask**. See
//!   [`in_process::fresh_env`] for how that rule is kept honest.
//!
//! Adding a test that needs `export`, `cd`/`pushd`, `umask` or an external command puts it in the
//! first module. Everything else goes in the second.

mod common;

/// Tests that mutate process-global state, each in its own process.
mod process_global {
    use crate::common::{assert_out, run, run_in};
    use std::os::unix::fs::PermissionsExt;

    /// `export` has to reach the real `environ`, which is only observable from a child.
    ///
    /// The in-process ancestor of this test asserted `env.get_param("BAZ") == Some("qux")` — a
    /// lookup in oslo's own `HashMap`, which would have passed just as happily if the `environ`
    /// write had been dropped entirely. Reading it back out of `env(1)` tests the thing that
    /// matters, and the negative half tests the other direction: an unexported assignment must
    /// *not* leak into a child.
    #[test]
    fn exported_assignments_reach_a_child_and_plain_ones_do_not() {
        let r = run(
            "OSLO_T_PLAIN=bar; export OSLO_T_EXPORTED=qux; echo \"$OSLO_T_PLAIN/$OSLO_T_EXPORTED\"; env",
        );
        assert_eq!(r.status, 0, "stderr: {}", r.stderr);

        let mut lines = r.out().lines();
        assert_eq!(
            lines.next(),
            Some("bar/qux"),
            "both variables must expand in the shell itself; stdout: {}",
            r.stdout
        );

        let environ: Vec<&str> = lines.collect();
        assert!(
            environ.contains(&"OSLO_T_EXPORTED=qux"),
            "exported variable missing from the child's environment: {:?}",
            environ
        );
        assert!(
            !environ.iter().any(|l| l.starts_with("OSLO_T_PLAIN")),
            "unexported variable leaked into the child's environment: {:?}",
            environ
        );
    }

    /// `export` on an already-set name promotes it without changing the value.
    #[test]
    fn export_of_an_existing_variable_promotes_it() {
        let r = run("OSLO_T_LATE=one; export OSLO_T_LATE; env");
        assert!(
            r.out().lines().any(|l| l == "OSLO_T_LATE=one"),
            "stdout: {}\nstderr: {}",
            r.stdout,
            r.stderr
        );
    }

    /// `unset` has to remove the name from `environ`, not just from the shell's table.
    #[test]
    fn unset_removes_an_exported_name_from_the_environment() {
        let r = run("export OSLO_T_GONE=1; unset OSLO_T_GONE; env");
        assert!(
            !r.out().lines().any(|l| l.starts_with("OSLO_T_GONE")),
            "stdout: {}\nstderr: {}",
            r.stdout,
            r.stderr
        );
    }

    /// The old test asserted only that `umask 0022` exited 0 — it never read the mask back, so a
    /// `umask` that parsed its argument and then threw it away would have passed.
    #[test]
    fn umask_reports_the_mask_it_was_given() {
        assert_out("umask 0022; umask", "0022");
        assert_out("umask 0077; umask", "0077");
        assert_out("umask 0002; umask -S", "u=rwx,g=rwx,o=rx");
    }

    /// The mask is only real if `open(2)` honours it. This is the assertion that would have
    /// caught a `umask` builtin that printed the right number without calling `umask(2)`.
    #[test]
    fn umask_applies_to_files_the_shell_creates() {
        let dir = tempfile::tempdir().expect("tempdir");
        let r = run_in(
            dir.path(),
            "umask 0077; echo hi > private; echo hi > second",
        );
        assert_eq!(r.status, 0, "stderr: {}", r.stderr);

        for name in ["private", "second"] {
            let mode = std::fs::metadata(dir.path().join(name))
                .expect("created file")
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600, "{name} should be 0666 & ~0077");
        }

        // A different mask in a different process must not see the previous one — the point of
        // running these out-of-process at all.
        let r = run_in(dir.path(), "umask 0022; echo hi > shared");
        assert_eq!(r.status, 0, "stderr: {}", r.stderr);
        let mode = std::fs::metadata(dir.path().join("shared"))
            .expect("created file")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o644);
    }

    /// `pushd` moves, `dirs` shows the stack, `popd` comes back.
    ///
    /// The in-process ancestor asserted the two exit statuses and nothing else, then restored the
    /// cwd by hand and hoped no other test had looked in between.
    #[test]
    fn pushd_and_popd_move_the_working_directory_and_back() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let start = tmp.path().canonicalize().expect("canonical tempdir");
        let target = start.join("target");
        std::fs::create_dir(&target).expect("create target dir");

        let r = run_in(
            &start,
            &format!(
                "pwd; pushd {t} >/dev/null; pwd; dirs; popd >/dev/null; pwd",
                t = target.display()
            ),
        );
        assert_eq!(r.status, 0, "stderr: {}", r.stderr);
        assert_eq!(
            r.lines(),
            vec![
                start.to_str().unwrap(),
                target.to_str().unwrap(),
                &format!("{} {}", target.display(), start.display()),
                start.to_str().unwrap(),
            ],
            "stderr: {}",
            r.stderr
        );
    }

    /// `popd` with an empty stack is an error, and must not move anywhere.
    #[test]
    fn popd_on_an_empty_stack_fails_without_moving() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let start = tmp.path().canonicalize().expect("canonical tempdir");

        let r = run_in(&start, "popd; echo \"status=$?\"; pwd");
        assert_eq!(
            r.lines(),
            vec!["status=1", start.to_str().unwrap()],
            "stderr: {}",
            r.stderr
        );
    }

    /// A command word naming a directory is a *failed command*, not a `cd` (PLAN R5.13).
    ///
    /// This test used to assert the opposite — status 0 — which is the bug: a script whose first
    /// line is `build` would change directory instead of reporting that `build` does not exist,
    /// and every relative path after it would resolve somewhere the author never intended. autocd
    /// is an interactive convenience, gated on both an interactive shell and an explicit opt-in
    /// (`oslo::exec::simple::set_autocd`), and `-c` is neither.
    #[test]
    fn auto_cd_is_off_outside_an_interactive_shell() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let start = tmp.path().canonicalize().expect("canonical tempdir");
        let target = start.join("target");
        std::fs::create_dir(&target).expect("create target dir");

        let bare = run_in(&start, target.to_str().unwrap());
        // 126, not 127: the name resolves to something that exists and cannot be executed.
        assert_eq!(bare.status, 126, "stderr: {}", bare.stderr);

        let after = run_in(
            &start,
            &format!("{t} 2>/dev/null; pwd", t = target.display()),
        );
        assert_eq!(
            after.out(),
            start.to_str().unwrap(),
            "the working directory must not have moved"
        );
    }
}

/// Tests that stay in process: build an AST, evaluate it, inspect `Environment`.
///
/// Every one of these is a pure function of the AST and the `Environment` it is handed. Nothing
/// here may write `environ`, `chdir` or `umask` — see the module docs at the top of the file.
mod in_process {
    use oslo::env::Environment;
    use oslo::exec::eval_command_list;
    use oslo::lua::LuaEngine;
    use oslo::parser::parse_bash_script;
    use std::sync::{Arc, Mutex};

    /// Every variable these tests assign carries this prefix.
    ///
    /// `Environment::new()` copies the real environment in and marks *everything it finds* as
    /// exported, so a later assignment to such a name goes on to call `unsafe { env::set_var }`.
    /// That is the one way an in-process test here could still write `environ`, and it would do
    /// so silently, depending on whoever's machine happened to export `X` or `COUNT`. A prefix no
    /// real environment uses closes that door, and [`fresh_env`] checks the door is shut.
    const VAR_PREFIX: &str = "OSLO_T_";

    /// The only way these tests are allowed to obtain an `Environment`.
    ///
    /// The assertion is the enforcement mechanism for [`VAR_PREFIX`]: if the ambient environment
    /// ever does contain such a name, this fails loudly at the top of the test instead of turning
    /// into a data race somewhere inside `set_var`.
    fn fresh_env() -> Environment {
        for (name, _) in std::env::vars() {
            assert!(
                !name.starts_with(VAR_PREFIX),
                "{name} is set in the real environment, so assigning to it in process would \
                 write `environ` from a test thread; unset it or rename the test variable"
            );
        }
        Environment::new()
    }

    fn run_cmd(env: &mut Environment, input: &str) -> i32 {
        let ast = parse_bash_script(input).expect("Parsing failed");
        eval_command_list(env, &ast).expect("Execution failed")
    }

    // Pipeline execution is deliberately *not* tested in process. Every pipeline stage is a
    // `fork()` (`exec/pipeline.rs`), and libtest runs these tests on a thread pool: a child forked
    // out of a multi-threaded parent inherits whatever locks the other threads held at that
    // instant, so it can deadlock in the allocator before reaching `execv` while the parent blocks
    // in `waitpid`. That made `cargo test` hang for roughly one run in twenty. It is a property of
    // the harness rather than of the shell — oslo itself never spawns a thread — so the pipeline
    // cases live in `expansion_tests.rs`, which spawns the real binary.

    #[test]
    fn plain_assignment_is_visible_to_expansion() {
        let mut env = fresh_env();
        let status = run_cmd(&mut env, "OSLO_T_FOO=bar; OSLO_T_ECHO=$OSLO_T_FOO");
        assert_eq!(status, 0);
        assert_eq!(env.get_param("OSLO_T_FOO"), Some("bar".to_string()));
        assert_eq!(env.get_param("OSLO_T_ECHO"), Some("bar".to_string()));
    }

    #[test]
    fn test_arithmetic_expansion() {
        let mut env = fresh_env();
        run_cmd(&mut env, "OSLO_T_X=10; OSLO_T_Y=$((OSLO_T_X + 5 * 2))");
        assert_eq!(env.get_param("OSLO_T_Y"), Some("20".to_string()));
    }

    #[test]
    fn test_if_else_compound_command() {
        let mut env = fresh_env();
        let status = run_cmd(
            &mut env,
            "if true; then OSLO_T_OUT=yes; else OSLO_T_OUT=no; fi",
        );
        assert_eq!(status, 0);
        assert_eq!(env.get_param("OSLO_T_OUT"), Some("yes".to_string()));
    }

    #[test]
    fn test_while_loop() {
        let mut env = fresh_env();
        run_cmd(
            &mut env,
            "OSLO_T_N=0; while [ $OSLO_T_N -lt 3 ]; do OSLO_T_N=$((OSLO_T_N + 1)); done",
        );
        assert_eq!(env.get_param("OSLO_T_N"), Some("3".to_string()));
    }

    #[test]
    fn test_for_loop() {
        let mut env = fresh_env();
        run_cmd(
            &mut env,
            "OSLO_T_V=\"\"; for i in a b c; do OSLO_T_V=\"$OSLO_T_V$i\"; done",
        );
        assert_eq!(env.get_param("OSLO_T_V"), Some("abc".to_string()));
    }

    #[test]
    fn test_lua_integration_exec() {
        let env = Arc::new(Mutex::new(fresh_env()));
        let lua = LuaEngine::new().expect("Lua init failed");
        lua.setup_bindings(Arc::clone(&env))
            .expect("Bindings failed");

        let script = r#"
            oslo.exec("OSLO_T_LUA=works")
            res = oslo.get_var("OSLO_T_LUA")
            oslo.set_alias("l", "ls -l")
            alias_val = oslo.get_alias("l")
        "#;

        lua.eval_script(script).expect("Script execution failed");
        let guard = env.lock().unwrap();
        assert_eq!(guard.get_param("OSLO_T_LUA"), Some("works".to_string()));
        assert_eq!(guard.get_alias("l"), Some("ls -l"));
    }

    #[test]
    fn test_bash_script_parsing() {
        let mut env = fresh_env();
        let script = r#"
            OSLO_T_A=20
            OSLO_T_B=30
            OSLO_T_C=$((OSLO_T_A + OSLO_T_B))
            if [ $OSLO_T_C -eq 50 ]; then
                OSLO_T_RESULT="success"
            fi
        "#;
        let status = run_cmd(&mut env, script);
        assert_eq!(status, 0);
        assert_eq!(env.get_param("OSLO_T_RESULT"), Some("success".to_string()));
    }

    #[test]
    fn test_eval_builtin() {
        let mut env = fresh_env();
        let status = run_cmd(&mut env, "eval 'OSLO_T_EVAL=100'");
        assert_eq!(status, 0);
        assert_eq!(env.get_param("OSLO_T_EVAL"), Some("100".to_string()));
    }

    /// `source` reads a file, so it needs a path — but not the *current directory*: the path is
    /// absolute, which is why this no longer takes a lock.
    #[test]
    fn test_source_builtin() {
        let mut env = fresh_env();
        let temp = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(temp.path(), "OSLO_T_SOURCED=hello").unwrap();
        let cmd = format!("source {}", temp.path().to_str().unwrap());
        let status = run_cmd(&mut env, &cmd);
        assert_eq!(status, 0);
        assert_eq!(env.get_param("OSLO_T_SOURCED"), Some("hello".to_string()));
    }

    #[test]
    fn test_builtin_conditions() {
        let mut env = fresh_env();
        assert_eq!(run_cmd(&mut env, "test -z ''"), 0);
        assert_eq!(run_cmd(&mut env, "[ 'abc' = 'abc' ]"), 0);
        assert_eq!(run_cmd(&mut env, "[ 10 -gt 5 ]"), 0);
    }

    /// `trap` only records the handler text here — installing the kernel handler happens
    /// elsewhere — so this one has no global side effect to isolate.
    #[test]
    fn test_trap_builtin() {
        let mut env = fresh_env();
        let status = run_cmd(&mut env, "trap 'echo cleanup' EXIT");
        assert_eq!(status, 0);
        assert_eq!(env.get_trap("EXIT"), Some("echo cleanup"));
    }

    #[test]
    fn test_readonly_vars() {
        let mut env = fresh_env();
        let status = run_cmd(&mut env, "readonly OSLO_T_IMMUTABLE=123");
        assert_eq!(status, 0);

        run_cmd(&mut env, "OSLO_T_IMMUTABLE=456");
        assert_eq!(env.get_param("OSLO_T_IMMUTABLE"), Some("123".to_string()));
    }

    #[test]
    fn test_function_local_scope() {
        let mut env = fresh_env();
        let script = r#"
            OSLO_T_G="outer"
            my_func() {
                local OSLO_T_G="inner"
            }
            my_func
        "#;
        let status = run_cmd(&mut env, script);
        assert_eq!(status, 0);
        assert_eq!(env.get_param("OSLO_T_G"), Some("outer".to_string()));
    }

    /// Reads the working directory but never changes it, which is only safe because no test in
    /// this binary changes it either — the cwd tests all live in `process_global`.
    #[test]
    fn test_interactive_prompt() {
        let left = oslo::interactive::prompt::render_default_left_prompt(0, "sh");
        assert!(!left.is_empty());
        // The right prompt was deleted in PLAN R9.7: nothing ever drew it, so this assertion was
        // the only thing keeping its renderer alive.
    }

    #[test]
    fn test_dropdown_menu_render() {
        let candidates = vec![
            oslo::interactive::dropdown::CompletionCandidate::new(
                "cargo".to_string(),
                "cargo".to_string(),
                Some("Rust package manager".to_string()),
            ),
            oslo::interactive::dropdown::CompletionCandidate::new(
                "cd".to_string(),
                "cd".to_string(),
                Some("Change working directory".to_string()),
            ),
        ];
        let (rendered, lines) =
            oslo::interactive::dropdown::render_vertical_dropdown(&candidates, 0, 8, 0, "");
        assert!(rendered.contains("cargo"));
        assert!(rendered.contains("cd"));
        // Two candidates, two rows: the menu has no border above or below them.
        assert_eq!(lines, 2);
    }
}

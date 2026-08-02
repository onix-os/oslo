//! The lifecycle, exercised against a real filesystem and a stand-in evaluator.
//!
//! A separate file rather than an inline module because `mod.rs` was over the 600-line limit and
//! this is the seam that costs nothing: what the module *does* and what proves it does it are
//! different subjects, and `src/interactive/tests.rs` is already laid out this way.

use super::*;

/// **Every test here must use variable names no other test uses.**
///
/// `set_var(.., export: true)` calls `environ_set`, which writes the *process* environment so
/// that children inherit it. libtest runs these in parallel and `Environment::new()` snapshots
/// that process environment, so one test exporting `A` puts `A` in another test's *before*
/// snapshot — the diff then records no change, unload has nothing to undo, and the failure
/// looks like a bug in this module rather than crosstalk between tests. It cost an afternoon
/// once already; the `OSLO_T_` prefix is the cheap fix.
fn shell() -> Mutex<Environment> {
    Mutex::new(Environment::new())
}

/// Read a variable the way a caller outside the lock would.
fn var(env: &Mutex<Environment>, name: &str) -> Option<String> {
    env.lock().unwrap().get_var(name).map(str::to_string)
}

/// A stand-in for the Lua engine, which the library cannot reach from here.
///
/// Reads the file as `NAME=VALUE` lines and exports them. These tests are about the lifecycle —
/// what loads, what unloads, what the allow gate refuses — and none of that depends on which
/// language did the setting. The real evaluator is exercised through the pty harness.
/// Built per test so it can reach the same environment the loader is diffing, taking the lock
/// itself — which is exactly what a real `.env.lua` does through `oslo.set_var`.
fn pairs_into(env: &Mutex<Environment>) -> impl FnMut(&Rc) -> Result<(), String> + '_ {
    move |rc: &Rc| {
        let source = std::fs::read_to_string(&rc.path).map_err(|e| e.to_string())?;
        let mut guard = env.lock().map_err(|_| "locked".to_string())?;
        for line in source.lines() {
            let line = line.trim();
            match line.strip_prefix("alias ") {
                Some(rest) => {
                    if let Some((name, value)) = rest.split_once('=') {
                        guard.set_alias(name.trim(), value.trim());
                    }
                }
                None => {
                    if let Some((name, value)) = line.split_once('=') {
                        guard.set_var(name.trim(), value.trim(), true);
                    }
                }
            }
        }
        Ok(())
    }
}

fn rc_in(dir: &Path, name: &str, body: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, body).expect("write");
    path
}

/// Nothing is read until it is allowed, and the notice is printed once.
#[test]
fn an_unallowed_file_is_not_read_and_is_reported_once() {
    let store = tempfile::tempdir().expect("temp dir");
    let project = tempfile::tempdir().expect("temp dir");
    rc_in(project.path(), find::NAME, "SECRET=leaked\n");

    let mut direnv = Direnv::new(store.path().to_str(), None);
    let env = shell();

    let events = direnv.arrive(&env, project.path(), &mut pairs_into(&env), &mut || {});
    assert!(matches!(events.as_slice(), [Event::Blocked { .. }]));
    assert_eq!(
        var(&env, "SECRET").as_deref(),
        None,
        "refused means not read"
    );

    // The second arrival says nothing: a warning shown on every prompt is a warning nobody
    // reads, which is the worst outcome for this particular warning.
    direnv.loaded = None;
    assert!(
        direnv
            .arrive(&env, project.path(), &mut pairs_into(&env), &mut || {})
            .is_empty()
    );
}

/// An alias a directory defines must not follow you out of it.
///
/// This is the gap that made the feature half-true for a while: variables were restored and
/// everything else a `.env.lua` set simply stayed. An alias is the common case — a project
/// defining `t` for its test command — and finding it still bound three directories later,
/// running the wrong project's tests, is exactly the kind of thing that makes a shell feel
/// haunted.
#[test]
fn an_alias_a_directory_defined_leaves_with_it() {
    let store = tempfile::tempdir().expect("temp dir");
    let project = tempfile::tempdir().expect("temp dir");
    let elsewhere = tempfile::tempdir().expect("temp dir");
    // The stand-in evaluator reads `NAME=VALUE`; `alias` lines it treats as aliases.
    let path = rc_in(project.path(), find::NAME, "alias t=cargo test\n");

    let env = shell();
    env.lock().unwrap().set_alias("keep", "echo untouched");
    let mut direnv = Direnv::new(store.path().to_str(), None);
    direnv.permissions().allow(&path).expect("allow");

    direnv.arrive(&env, project.path(), &mut pairs_into(&env), &mut || {});
    assert_eq!(
        env.lock()
            .unwrap()
            .get_aliases()
            .get("t")
            .map(String::as_str),
        Some("cargo test")
    );

    direnv.arrive(&env, elsewhere.path(), &mut pairs_into(&env), &mut || {});
    assert_eq!(
        env.lock().unwrap().get_aliases().get("t"),
        None,
        "leaving must take the alias with it"
    );
    assert_eq!(
        env.lock()
            .unwrap()
            .get_aliases()
            .get("keep")
            .map(String::as_str),
        Some("echo untouched"),
        "and must not touch an alias it did not set"
    );
}

/// A shell-local variable that a directory exports must come back *local*, not vanish.
///
/// The snapshot used to be `exported_vars()` alone, which cannot tell "was not here" from "was here
/// but local". So a variable you had set without exporting, that a `.env.lua` then exported, was
/// removed entirely on the way out — the directory environment quietly deleting something it did
/// not create.
#[test]
fn a_local_variable_the_directory_exported_comes_back_local() {
    let store = tempfile::tempdir().expect("temp dir");
    let project = tempfile::tempdir().expect("temp dir");
    let elsewhere = tempfile::tempdir().expect("temp dir");
    let path = rc_in(project.path(), find::NAME, "OSLO_T_LOCAL=exported\n");

    let env = shell();
    // Set, but deliberately not exported.
    env.lock().unwrap().set_var("OSLO_T_LOCAL", "mine", false);
    let mut direnv = Direnv::new(store.path().to_str(), None);
    direnv.permissions().allow(&path).expect("allow");

    direnv.arrive(&env, project.path(), &mut pairs_into(&env), &mut || {});
    assert_eq!(var(&env, "OSLO_T_LOCAL").as_deref(), Some("exported"));

    direnv.arrive(&env, elsewhere.path(), &mut pairs_into(&env), &mut || {});
    assert_eq!(
        var(&env, "OSLO_T_LOCAL").as_deref(),
        Some("mine"),
        "the value has to come back"
    );
    let exported = env
        .lock()
        .unwrap()
        .exported_vars()
        .into_iter()
        .any(|(name, _)| name == "OSLO_T_LOCAL");
    assert!(
        !exported,
        "and it has to come back shell-local, not exported"
    );
}

/// `PATH` is not special-cased, and that is the point worth pinning.
///
/// It is an exported variable like any other, so extending it in a `.env.lua` and having the
/// extension disappear on the way out falls out of the same diff that handles everything else.
/// What makes it more than bookkeeping is that restoring it goes through `set_var`, which flushes
/// the command-location cache — so a tool that was only on the project's `PATH` stops resolving the
/// instant you leave, rather than still being found through a stale hash.
#[test]
fn a_path_a_project_added_does_not_follow_you_out() {
    let store = tempfile::tempdir().expect("temp dir");
    let project = tempfile::tempdir().expect("temp dir");
    let elsewhere = tempfile::tempdir().expect("temp dir");
    let path = rc_in(project.path(), find::NAME, "PATH=/opt/acme/bin:/usr/bin\n");

    let env = shell();
    env.lock().unwrap().set_var("PATH", "/usr/bin", true);
    let mut direnv = Direnv::new(store.path().to_str(), None);
    direnv.permissions().allow(&path).expect("allow");

    direnv.arrive(&env, project.path(), &mut pairs_into(&env), &mut || {});
    assert_eq!(var(&env, "PATH").as_deref(), Some("/opt/acme/bin:/usr/bin"));

    direnv.arrive(&env, elsewhere.path(), &mut pairs_into(&env), &mut || {});
    assert_eq!(
        var(&env, "PATH").as_deref(),
        Some("/usr/bin"),
        "the project's directory must come back off the front"
    );
}

/// The whole point: arriving sets, leaving restores.
#[test]
fn arriving_loads_and_leaving_puts_everything_back() {
    let store = tempfile::tempdir().expect("temp dir");
    let project = tempfile::tempdir().expect("temp dir");
    let elsewhere = tempfile::tempdir().expect("temp dir");
    let path = rc_in(
        project.path(),
        find::NAME,
        "DATABASE_URL=postgres://local\n",
    );

    let mut direnv = Direnv::new(store.path().to_str(), None);
    direnv.permissions().allow(&path).expect("allow");
    let env = shell();
    env.lock().unwrap().set_var("EDITOR", "vim", true);

    direnv.arrive(&env, project.path(), &mut pairs_into(&env), &mut || {});
    assert_eq!(
        var(&env, "DATABASE_URL").as_deref(),
        Some("postgres://local")
    );

    direnv.arrive(&env, elsewhere.path(), &mut pairs_into(&env), &mut || {});
    assert_eq!(
        var(&env, "DATABASE_URL").as_deref(),
        None,
        "leaving must remove it, not blank it"
    );
    assert_eq!(
        var(&env, "EDITOR").as_deref(),
        Some("vim"),
        "and touch nothing else"
    );
}

/// Moving straight from one project to another must not merge them.
#[test]
fn one_project_never_leaks_into_the_next() {
    let store = tempfile::tempdir().expect("temp dir");
    let first = tempfile::tempdir().expect("temp dir");
    let second = tempfile::tempdir().expect("temp dir");
    let one = rc_in(first.path(), find::NAME, "ONLY_IN_FIRST=1\n");
    let two = rc_in(second.path(), find::NAME, "ONLY_IN_SECOND=2\n");

    let mut direnv = Direnv::new(store.path().to_str(), None);
    direnv.permissions().allow(&one).expect("allow");
    direnv.permissions().allow(&two).expect("allow");
    let env = shell();

    direnv.arrive(&env, first.path(), &mut pairs_into(&env), &mut || {});
    direnv.arrive(&env, second.path(), &mut pairs_into(&env), &mut || {});

    assert_eq!(var(&env, "ONLY_IN_SECOND").as_deref(), Some("2"));
    assert_eq!(
        var(&env, "ONLY_IN_FIRST").as_deref(),
        None,
        "unload has to happen before load, or the two environments merge"
    );
}

/// Standing still costs nothing, which is what makes this affordable on every `cd`.
#[test]
fn staying_put_does_no_work() {
    let store = tempfile::tempdir().expect("temp dir");
    let project = tempfile::tempdir().expect("temp dir");
    let path = rc_in(project.path(), find::NAME, "OSLO_T_STAY=1\n");

    let mut direnv = Direnv::new(store.path().to_str(), None);
    direnv.permissions().allow(&path).expect("allow");
    let env = shell();

    assert!(
        !direnv
            .arrive(&env, project.path(), &mut pairs_into(&env), &mut || {})
            .is_empty()
    );
    assert!(
        direnv
            .arrive(&env, project.path(), &mut pairs_into(&env), &mut || {})
            .is_empty(),
        "a second arrival in the same place is not a reload"
    );
}

/// Denying a *loaded* environment must take its variables back out.
///
/// The bug this pins: an early version dropped the loaded record on a decision, and the undo
/// diff is the only thing that knows how to remove the variables — so a denied environment
/// stayed applied with nothing able to unload it. Marking the record stale instead means the
/// next arrival unloads properly first.
#[test]
fn denying_what_is_loaded_unloads_it() {
    let store = tempfile::tempdir().expect("temp dir");
    let project = tempfile::tempdir().expect("temp dir");
    let path = rc_in(project.path(), find::NAME, "OSLO_T_DENY=1\n");

    let mut direnv = Direnv::new(store.path().to_str(), None);
    direnv.permissions().allow(&path).expect("allow");
    let env = shell();
    direnv.arrive(&env, project.path(), &mut pairs_into(&env), &mut || {});
    assert_eq!(var(&env, "OSLO_T_DENY").as_deref(), Some("1"));

    direnv.permissions().deny(&path).expect("deny");
    direnv.invalidate();

    // Standing in the very same directory, which the early-return would otherwise skip.
    let events = direnv.arrive(&env, project.path(), &mut pairs_into(&env), &mut || {});
    assert!(
        events.iter().any(|e| matches!(e, Event::Unloaded { .. })),
        "the record has to survive long enough to be undone: {events:?}"
    );
    assert_eq!(
        var(&env, "OSLO_T_DENY").as_deref(),
        None,
        "a denied environment must not stay applied"
    );
}

/// Allowing takes effect where you are standing, not on the next `cd`.
#[test]
fn allowing_loads_without_moving() {
    let store = tempfile::tempdir().expect("temp dir");
    let project = tempfile::tempdir().expect("temp dir");
    let path = rc_in(project.path(), find::NAME, "OSLO_T_NOW=1\n");

    let mut direnv = Direnv::new(store.path().to_str(), None);
    let env = shell();
    direnv.arrive(&env, project.path(), &mut pairs_into(&env), &mut || {});
    assert_eq!(
        var(&env, "OSLO_T_NOW").as_deref(),
        None,
        "blocked until allowed"
    );

    direnv.permissions().allow(&path).expect("allow");
    direnv.invalidate();
    direnv.arrive(&env, project.path(), &mut pairs_into(&env), &mut || {});
    assert_eq!(
        var(&env, "OSLO_T_NOW").as_deref(),
        Some("1"),
        "`direnv allow` has to work where you already are"
    );
}

/// A subdirectory of the project is still the project.
#[test]
fn walking_deeper_stays_loaded() {
    let store = tempfile::tempdir().expect("temp dir");
    let project = tempfile::tempdir().expect("temp dir");
    let deep = project.path().join("src/inner");
    std::fs::create_dir_all(&deep).expect("mkdir");
    let path = rc_in(project.path(), find::NAME, "OSLO_T_DEEP=1\n");

    let mut direnv = Direnv::new(store.path().to_str(), None);
    direnv.permissions().allow(&path).expect("allow");
    let env = shell();

    direnv.arrive(&env, project.path(), &mut pairs_into(&env), &mut || {});
    assert!(
        direnv
            .arrive(&env, &deep, &mut pairs_into(&env), &mut || {})
            .is_empty()
    );
    assert_eq!(var(&env, "OSLO_T_DEEP").as_deref(), Some("1"));
}

/// Editing an allowed file revokes it, so the next arrival must refuse rather than reload.
#[test]
fn an_edit_revokes_and_the_environment_comes_back_out() {
    let store = tempfile::tempdir().expect("temp dir");
    let project = tempfile::tempdir().expect("temp dir");
    let path = rc_in(project.path(), find::NAME, "OSLO_T_EDIT_A=1\n");

    let mut direnv = Direnv::new(store.path().to_str(), None);
    direnv.permissions().allow(&path).expect("allow");
    let env = shell();
    direnv.arrive(&env, project.path(), &mut pairs_into(&env), &mut || {});
    assert_eq!(var(&env, "OSLO_T_EDIT_A").as_deref(), Some("1"));

    // Rewrite it. The mtime moves, so the next arrival re-checks, and the hash no longer matches.
    std::thread::sleep(std::time::Duration::from_millis(10));
    rc_in(
        project.path(),
        find::NAME,
        "OSLO_T_EDIT_A=1\nOSLO_T_EDIT_B=2\n",
    );

    let events = direnv.arrive(&env, project.path(), &mut pairs_into(&env), &mut || {});
    assert!(
        events.iter().any(|e| matches!(e, Event::Blocked { .. })),
        "an edited file has to be allowed again: {events:?}"
    );
    assert_eq!(
        var(&env, "OSLO_T_EDIT_A").as_deref(),
        None,
        "and the old values come back out"
    );
    assert_eq!(
        var(&env, "OSLO_T_EDIT_B").as_deref(),
        None,
        "the new ones never went in"
    );
}

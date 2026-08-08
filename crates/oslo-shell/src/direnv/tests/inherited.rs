//! What a shell inherits, and how it gives it back.
//!
//! Separate from `direnv/tests.rs` because it is a different subject: those tests are about one
//! shell walking between directories, these are about two shells and what passes between them
//! through `environ`. See [`crate::direnv::carry`].

use super::super::*;
use super::{feature_unchanged, pairs_into, rc_in, shell, var};

/// **A shell that inherited a directory environment can give it back.**
///
/// This is the leak that made every session look haunted. The variables have to go into the real
/// `environ` for a child to see them at all, so a new pane, a nested `oslo` or an editor's terminal
/// spawned from inside a project inherits everything that project set. Before the record travelled
/// with them, that child started believing nothing was loaded: it carried one repository's `$PATH`,
/// its `TOP_HEAD` and its exported variables into every directory it ever visited, and no `cd`
/// could shift them because there was nothing recorded to put back.
#[test]
fn an_inherited_environment_is_unloaded_by_the_shell_that_inherits_it() {
    let _feature = feature_unchanged();
    let store = tempfile::tempdir().expect("temp dir");
    let project = tempfile::tempdir().expect("temp dir");
    let elsewhere = tempfile::tempdir().expect("temp dir");
    let path = rc_in(project.path(), find::NAME, "OSLO_T_CARRY_OUT=A\n");

    // The parent: standing in the project, with its environment loaded.
    let parent_env = shell();
    let mut parent = Direnv::adopting(store.path().to_str(), None, None);
    parent.permissions().allow(&path).expect("allow");
    parent.arrive(
        &parent_env,
        project.path(),
        &mut pairs_into(&parent_env),
        &mut || {},
    );
    assert_eq!(var(&parent_env, "OSLO_T_CARRY_OUT").as_deref(), Some("A"));
    let carried = var(&parent_env, carry::NAME).expect("the record is exported for children");

    // The child: a fresh shell, holding the variables because it inherited them, and standing
    // somewhere else entirely.
    let child_env = shell();
    child_env
        .lock()
        .unwrap()
        .set_var("OSLO_T_CARRY_OUT", "A", true);
    let mut child = Direnv::adopting(store.path().to_str(), None, Some(&carried));
    assert_eq!(
        child.active(),
        Some(project.path()),
        "it should know what it is holding"
    );
    child.arrive(
        &child_env,
        elsewhere.path(),
        &mut pairs_into(&child_env),
        &mut || {},
    );
    assert_eq!(
        var(&child_env, "OSLO_T_CARRY_OUT"),
        None,
        "the inherited variable must not follow it out"
    );
    assert_eq!(
        var(&child_env, carry::NAME),
        None,
        "and neither must the record, or the next child undoes it twice"
    );
}

/// **A child that starts *in* the same project runs the file itself.**
///
/// `execve` carries variables and nothing else, so such a child holds the project's `$PATH` and has
/// none of its aliases, prompt or functions — the Lua half of the file never ran there. Counting
/// that as loaded is what left `_b` reporting `command not found` in a shell whose `$PATH` was
/// already the project's, with no `cd` able to fix it because the owner always matched.
///
/// Re-running is only safe because it unloads first, which is the point this guards: the variable
/// has to come back with the value the file sets, not that value applied on top of the inherited
/// one. That doubling is what the old keep-it-as-is behaviour was written to avoid.
#[test]
fn a_child_that_starts_in_the_project_runs_the_file_itself() {
    let _feature = feature_unchanged();
    let store = tempfile::tempdir().expect("temp dir");
    let project = tempfile::tempdir().expect("temp dir");
    let elsewhere = tempfile::tempdir().expect("temp dir");
    let path = rc_in(
        project.path(),
        find::NAME,
        "OSLO_T_CARRY_IN=A\nalias _b=make build\n",
    );

    let parent_env = shell();
    let mut parent = Direnv::adopting(store.path().to_str(), None, None);
    parent.permissions().allow(&path).expect("allow");
    parent.arrive(
        &parent_env,
        project.path(),
        &mut pairs_into(&parent_env),
        &mut || {},
    );
    let carried = var(&parent_env, carry::NAME).expect("a record");

    let child_env = shell();
    child_env
        .lock()
        .unwrap()
        .set_var("OSLO_T_CARRY_IN", "A", true);
    let mut child = Direnv::adopting(store.path().to_str(), None, Some(&carried));
    assert_eq!(
        child_env.lock().unwrap().get_aliases().get("_b"),
        None,
        "an alias is not something a child can inherit"
    );
    child.arrive(
        &child_env,
        project.path(),
        &mut pairs_into(&child_env),
        &mut || {},
    );
    assert_eq!(
        child_env.lock().unwrap().get_aliases().get("_b").cloned(),
        Some("make build".to_string()),
        "so the child has to run the file to get one"
    );
    assert_eq!(
        var(&child_env, "OSLO_T_CARRY_IN").as_deref(),
        Some("A"),
        "and the variable it did inherit is set once, not applied over itself"
    );

    child.arrive(
        &child_env,
        elsewhere.path(),
        &mut pairs_into(&child_env),
        &mut || {},
    );
    assert_eq!(
        var(&child_env, "OSLO_T_CARRY_IN"),
        None,
        "and it still leaves properly"
    );
    assert_eq!(
        child_env.lock().unwrap().get_aliases().get("_b"),
        None,
        "taking with it the alias it ran itself"
    );
}

/// A record that is not one is ignored, and the shell simply believes nothing is loaded. It
/// arrives from the environment, which is to say from anywhere.
#[test]
fn a_corrupt_record_is_ignored() {
    let store = tempfile::tempdir().expect("temp dir");
    for bad in ["", "nonsense", "1 4:/tmp 9:short"] {
        let direnv = Direnv::adopting(store.path().to_str(), None, Some(bad));
        assert_eq!(direnv.active(), None, "{bad:?}");
    }
}

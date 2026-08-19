//! The table's shape. What each call *reads* is `oslo_ui::git`'s subject and is tested there.

use super::super::util::probe;
use super::*;

#[test]
fn every_name_is_on_the_table() {
    let Value::Table(built) = build() else {
        panic!("not a table")
    };
    let built = built.borrow();
    for name in [
        "branch",
        "root",
        "dir",
        "head",
        "operation",
        "stash",
        "upstream",
        "tag",
    ] {
        assert!(
            matches!(built.get_str(name), Value::Function(_)),
            "no {name}"
        );
    }
}

/// **Outside a repository nothing raises**, which is the whole contract: a prompt runs everywhere,
/// including in `/tmp`, and a call that raised there would break the prompt rather than the repo.
#[test]
fn outside_a_repository_every_call_answers() {
    let elsewhere = tempfile::tempdir().expect("temp dir");
    let held = std::env::current_dir().expect("cwd");
    // A repository above the temp directory would make this test assert nothing, so the answers
    // are only checked when there genuinely is none.
    std::env::set_current_dir(elsewhere.path()).expect("cd");
    let outside = oslo_ui::prompt::git_root_of(elsewhere.path()).is_none();

    let Value::Table(built) = build() else {
        panic!("not a table")
    };
    let built = built.borrow();
    for name in [
        "branch",
        "root",
        "dir",
        "head",
        "operation",
        "upstream",
        "tag",
    ] {
        let answered = probe::call(&built.get_str(name), Vec::new());
        match answered {
            Ok(values) if outside => assert!(
                matches!(values.first(), None | Some(Value::Nil)),
                "{name} answered something outside a repository"
            ),
            Ok(_) => {}
            Err(e) => panic!("{name} raised: {e}"),
        }
    }
    // `stash` is a count either way — "nothing stashed" and "not a repository" are the same answer
    // to `> 0`, and a prompt should not have to tell them apart.
    assert!(matches!(
        probe::first(&built.get_str("stash"), Vec::new()),
        Value::Number(_)
    ));

    std::env::set_current_dir(held).expect("cd back");
}

//! The shell side of a peer's `cd`.
//!
//! **One test, and it has to be.** The request slot is process-wide and so is the working directory
//! this moves; libtest runs test functions on threads, so a second one drains the first one's slot
//! and moves its ground. Split in two, they failed each other.

use super::*;
use crate::lua::api::live::queued;

#[test]
fn a_queued_move_is_made_once_and_carries_the_variables_with_it() {
    let env = Arc::new(Mutex::new(Environment::new()));
    let start = std::env::current_dir().expect("a working directory");
    assert!(queued::arm());
    let _ = queued::take();

    assert!(!follow(&env), "nothing asked for, nothing done");

    let target = std::fs::canonicalize("/tmp").expect("/tmp resolves");
    assert!(queued::ask(target.clone()));
    assert!(follow(&env), "the move was made");
    assert_eq!(
        std::env::current_dir().expect("a working directory"),
        target,
        "the process really moved"
    );

    // **Through `cd`, not through `set_current_dir`.** These are what a second route would have
    // quietly left behind, and a shell whose `$PWD` disagrees with its prompt is worse than one
    // that did not move at all.
    {
        let held = env.lock().expect("the environment");
        assert_eq!(
            held.get_var("PWD").map(str::to_string),
            Some(target.to_string_lossy().into_owned()),
            "$PWD followed"
        );
        assert!(held.get_var("OLDPWD").is_some(), "$OLDPWD was written");
    }

    assert!(!follow(&env), "and the slot is empty again");

    // **Held state is a command running, and one of those is a file browser.** The full move needs
    // the shell state, which `builtin_nav` holds for as long as the browser is up — so the kernel's
    // idea of where we are moves now, which is what a prompt reads, and the rest stays owed.
    std::env::set_current_dir(&start).expect("back for the second half");
    assert!(queued::ask(target.clone()));
    {
        let _held = env.lock().expect("exactly what a running builtin holds");
        assert!(
            follow(&env),
            "the kernel moved even though the state is held"
        );
        assert_eq!(
            std::env::current_dir().expect("a working directory"),
            target,
            "which is what the prompt reads"
        );
    }

    // **And it is still owed.** `$PWD`, the ring and `post-change-dir` have not happened, so the
    // request survives for the next safe point rather than being spent on the half that could be
    // done early.
    assert!(follow(&env), "the request was left for the safe point");

    // Put the process back: every other test in this binary shares this directory.
    std::env::set_current_dir(&start).expect("back to where the test started");
}

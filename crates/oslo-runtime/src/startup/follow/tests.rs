//! The shell side of a peer's `cd`.
//!
//! **One test, for the reason `queued`'s own tests are one**: the slot is process-wide, and so is
//! the working directory this moves. Two of these on two threads would move each other's ground.

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
            held.get_var("PWD").map(|pwd| pwd.to_string()),
            Some(target.to_string_lossy().into_owned()),
            "$PWD followed"
        );
        assert!(held.get_var("OLDPWD").is_some(), "$OLDPWD was written");
    }

    assert!(!follow(&env), "and the slot is empty again");

    // Put the process back: every other test in this binary shares this directory.
    std::env::set_current_dir(&start).expect("back to where the test started");
}

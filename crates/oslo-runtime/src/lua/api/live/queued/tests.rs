//! The handover, without a shell around it.
//!
//! **One test, deliberately.** The slot and the pipe are process-wide — there is one shell per
//! process and so one of each — and libtest runs test functions on threads. Split into three cases
//! they raced each other and two of them failed, which is the same in-process global-state trap
//! `tests/common` records for `environ` and the working directory.

use super::*;

#[test]
fn the_slot_holds_the_last_word_and_gives_it_up_once() {
    assert!(arm(), "the pipe opens");
    assert!(arm(), "and opening it twice is the same pipe");
    let _ = take();

    // **Taken once.** A slot that answered the same directory for ever would move the shell back
    // there on every wake, because the servicer runs on all of them.
    assert_eq!(take(), None, "nothing asked for");
    assert!(ask(PathBuf::from("/tmp")));
    assert_eq!(take(), Some(PathBuf::from("/tmp")));
    assert_eq!(take(), None, "and it is gone");

    // **The last word wins.** A backlog would send the shell through directories nobody is looking
    // at on the way to the one that was wanted, firing `post-change-dir` at each.
    assert!(ask(PathBuf::from("/tmp")));
    assert!(ask(PathBuf::from("/usr")));
    assert_eq!(take(), Some(PathBuf::from("/usr")));
    assert_eq!(take(), None);

    // **Asking never blocks.** The server thread is answering a peer that is waiting on it, so a
    // write to a pipe nobody has drained must fall through rather than park. Far more asks than
    // the pipe holds, since no shell is here to empty it.
    for _ in 0..2048 {
        assert!(ask(PathBuf::from("/tmp")), "an ask must always answer");
    }
    assert_eq!(take(), Some(PathBuf::from("/tmp")));
}

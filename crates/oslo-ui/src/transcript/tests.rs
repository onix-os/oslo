//! The number, and what may not be it.

use super::*;

/// **A number a terminal already acts on is refused, whatever asked for it.** Claiming one does not
/// add a mark, it takes away what that number did — and silently, far from the config line that
/// caused it. `7` is the working directory a mux reads; `133` is the marks this sits beside.
#[test]
fn a_reserved_number_is_never_used() {
    for taken in [0, 1, 2, 4, 5, 7, 9, 99, 104, 105, 133, 777, 1337] {
        assert!(!usable(taken), "{taken} is somebody else's");
    }
    for free in [DEFAULT_OSC, 1441, 2000] {
        assert!(usable(free), "{free} is not claimed by anything");
    }
    // hexe's palette protocol is 1330 and oslo must not sit on it, but nothing here reserves it:
    // it is hexe's own number, not a terminal's, so a config on a machine without hexe may use it.
    assert!(usable(1330));
}

/// The default is next door to `OSC 133`, whose region a transcript sits inside.
#[test]
fn the_default_is_next_to_the_marks_it_belongs_with() {
    assert_eq!(DEFAULT_OSC, 1440);
    assert!(usable(DEFAULT_OSC));
}

/// **Silent when there is nothing to mark.** A script, a pipe, a `-c` — a program reading oslo's
/// output must never find an escape sequence the shell invented in it, and that rule is older than
/// this file.
#[test]
fn nothing_is_written_when_marks_are_off() {
    crate::marks::enable(false);
    assert_eq!(mark(true), "");
    assert_eq!(mark(false), "");
}

/// **A cleared screen skips the prompt's leading blank.** `clear` puts the cursor at row one; a
/// blank written there spends the space the clear was asked for.
#[test]
fn a_clearing_command_is_recognised_and_answered_once() {
    for blanks in ["clear", "reset", "tput clear", "tput reset", "  clear  "] {
        ran(blanks);
        assert!(cleared(), "{blanks} blanks the screen");
    }
    for keeps in [
        "ls",
        "echo clear",
        "clear-cache",
        "git reset --hard",
        "tput cols",
        "",
    ] {
        ran(keeps);
        assert!(!cleared(), "{keeps} does not");
    }

    // **Answered once.** It is true of the prompt that follows the clear and of no other, so the
    // answer is taken rather than read — otherwise every prompt after a `clear` would lose its row.
    ran("clear");
    assert!(cleared());
    assert!(!cleared(), "the next prompt gets its blank back");
}

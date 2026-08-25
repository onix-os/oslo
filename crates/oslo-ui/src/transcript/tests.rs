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
///
/// **A fact about the screen, not about the last command**, which is the second half below. Set
/// from the command name alone it was wrong in both directions, and both showed as the spacing
/// changing on its own: it stayed `true` across a blank Enter and a `Ctrl-C` — neither runs a
/// command — so every prompt after a `clear` went on skipping its blank row until something real
/// was typed; and it stayed `false` through `Ctrl-L`, the one blank screen oslo does not have to
/// guess about, so the prompt landed on row two of a screen just cleared to get it to row one.
///
/// One test, because the flag is process-wide and libtest runs test functions on threads: split in
/// two they would take each other's answer.
#[test]
fn a_clearing_command_is_recognised() {
    for blanks in ["clear", "reset", "tput clear", "tput reset", "  clear  "] {
        ran(blanks);
        assert!(blank_now(), "{blanks} blanks the screen");
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
        assert!(!blank_now(), "{keeps} does not");
    }

    // **Read, not taken.** `lead` asks once per drawn frame, not once per prompt — so consuming the
    // answer would give the first frame no blank row and the next keystroke one, and the prompt
    // would grow a row under the cursor as soon as you typed. The next *command* clears it.
    ran("clear");
    assert!(blank_now());
    assert!(
        blank_now(),
        "still true on the next frame of the same prompt"
    );
    ran("ls");
    assert!(!blank_now(), "and false once something else has run");

    // The other two writers, which a command name cannot speak for.
    ran("clear");
    // What a line nobody ran leaves behind: the editor stepped the cursor past the block.
    wrote();
    assert!(!blank_now(), "a blank Enter is still a row used");
    // And what the editor does itself, which needs no guessing at all.
    blanked();
    assert!(blank_now(), "Ctrl-L cleared it");
    wrote();
}

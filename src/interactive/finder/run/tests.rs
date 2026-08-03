//! The key map and the scrolling, which are the two things a person notices immediately.
//!
//! The loop itself needs a terminal and is not tested here; what is testable is everything it
//! decides — what a keypress means, and where the window ends up after it.

use super::*;

fn command(line: &str, last_at: i64) -> Command {
    Command {
        line: line.to_string(),
        mode: "sh".to_string(),
        runs: 1,
        last_at,
        dir: "/here".to_string(),
        places: 1,
        worked: true,
    }
}

fn many(n: usize) -> Vec<Command> {
    (0..n)
        .map(|i| command(&format!("command-{i:03}"), (n - i) as i64))
        .collect()
}

/// Down moves toward the search bar, which is where the best match is. The list grows upward, so
/// this is the direction the arrow points on screen — getting it backwards is the most noticeable
/// thing a finder can get wrong.
#[test]
fn the_arrows_point_the_way_the_list_grows() {
    assert_eq!(key(b"\x1b[A"), Key::Up);
    assert_eq!(key(b"\x1b[B"), Key::Down);
    // The application-cursor spelling a terminal switches to inside a full-screen program.
    assert_eq!(key(b"\x1bOA"), Key::Up);
    assert_eq!(key(b"\x1bOB"), Key::Down);
    assert_eq!(key(b"\x10"), Key::Up);
    assert_eq!(key(b"\x0e"), Key::Down);
}

#[test]
fn the_keys_that_leave() {
    assert_eq!(key(b"\x1b"), Key::Cancel);
    assert_eq!(key(b"\x03"), Key::Cancel);
    assert_eq!(key(b"\r"), Key::Accept);
    assert_eq!(key(b"\n"), Key::Accept);
}

#[test]
fn the_keys_that_edit_the_query() {
    assert_eq!(key(b"\x7f"), Key::Backspace);
    assert_eq!(key(b"\x08"), Key::Backspace);
    assert_eq!(key(b"\x15"), Key::Clear);
    assert_eq!(key(b"g"), Key::Char('g'));
    assert_eq!(key(b" "), Key::Char(' '));
    assert_eq!(key(b"-"), Key::Char('-'));
}

/// A terminal delivers a multibyte character in one read. Dropping it would make the finder
/// unusable for anyone whose commands are not ASCII.
#[test]
fn a_multibyte_character_is_one_keypress() {
    assert_eq!(key("é".as_bytes()), Key::Char('é'));
    assert_eq!(key("→".as_bytes()), Key::Char('→'));
}

/// An escape sequence nothing maps to must do nothing, not insert its bytes into the query.
#[test]
fn an_unknown_sequence_is_ignored() {
    assert_eq!(key(b"\x1b[Z"), Key::Ignored);
    assert_eq!(key(b"\x1b[200~"), Key::Ignored);
    assert_eq!(key(&[]), Key::Ignored);
    // A control character that is not one of ours is not query text.
    assert_eq!(key(b"\x01"), Key::Ignored);
}

#[test]
fn page_keys_move_a_window_at_a_time() {
    assert_eq!(key(b"\x1b[5~"), Key::PageUp);
    assert_eq!(key(b"\x1b[6~"), Key::PageDown);
}

/// The selection cannot leave the list, however hard a key is held.
#[test]
fn the_selection_is_clamped() {
    let commands = many(5);
    let mut state = State::new(&commands, "/here", Fuzzy::Smart);
    state.fit(10);
    state.move_by(-100);
    assert_eq!(state.selected, 0);
    state.move_by(100);
    assert_eq!(state.selected, 4);
    state.move_by(1);
    assert_eq!(state.selected, 4, "already at the end");
}

/// A list longer than the window scrolls, and the selection stays on screen.
#[test]
fn the_window_follows_the_selection() {
    let commands = many(100);
    let mut state = State::new(&commands, "/here", Fuzzy::Smart);
    // 12 rows: 10 for the list.
    state.fit(12);
    assert_eq!(state.window, 10);

    state.move_by(9);
    assert_eq!(state.selected, 9);
    assert_eq!(state.offset, 0, "still in the first window");

    state.move_by(1);
    assert_eq!(state.selected, 10);
    assert_eq!(state.offset, 1, "scrolled by one");

    state.move_by(-10);
    assert_eq!(state.selected, 0);
    assert_eq!(state.offset, 0, "scrolled back");
}

/// The window never runs past the end of the list, which would draw blank rows between the last
/// match and the search bar and read as the list having ended early.
#[test]
fn the_window_does_not_overrun_the_list() {
    let commands = many(12);
    let mut state = State::new(&commands, "/here", Fuzzy::Smart);
    state.fit(12);
    state.move_by(100);
    assert_eq!(state.selected, 11);
    assert_eq!(state.offset, 2, "the last window, not past it");
}

/// Typing resets the selection: the old index referred to a list that no longer exists, and
/// keeping it would leave the cursor on an unrelated command.
#[test]
fn filtering_returns_to_the_top() {
    let commands = many(50);
    let mut state = State::new(&commands, "/here", Fuzzy::Smart);
    state.fit(12);
    state.move_by(20);
    assert_eq!(state.selected, 20);

    state.query.push_str("command-0");
    state.refilter();
    assert_eq!(state.selected, 0);
    assert_eq!(state.offset, 0);
    assert!(!state.matches.is_empty());
}

/// A resize between frames must not leave the selection off screen.
#[test]
fn shrinking_the_terminal_keeps_the_selection_visible() {
    let commands = many(100);
    let mut state = State::new(&commands, "/here", Fuzzy::Smart);
    state.fit(40);
    state.move_by(30);
    assert_eq!(state.selected, 30);

    state.fit(7);
    assert_eq!(state.window, 5);
    assert!(
        state.selected >= state.offset && state.selected < state.offset + state.window,
        "selected {} is outside the window at {}..{}",
        state.selected,
        state.offset,
        state.offset + state.window
    );
}

/// A query that matches nothing leaves an empty list, and moving in it must not panic.
#[test]
fn an_empty_result_list_is_safe_to_move_in() {
    let commands = many(5);
    let mut state = State::new(&commands, "/here", Fuzzy::Smart);
    state.query.push_str("zzzzzzzz-no-such-command");
    state.refilter();
    assert!(state.matches.is_empty());
    state.fit(20);
    state.move_by(1);
    state.move_by(-1);
    assert_eq!(state.selected, 0);
}

//! The finder's own state: where the selection is, and where the window follows it to.
//!
//! What a keypress *means* is no longer decided here — the reader moved to
//! [`crate::interactive::term`] when the finder stopped being the only thing that needed one, and
//! its tests went with it.

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

/// Index zero is drawn nearest the search bar. Up must advance to the row above it and Down must
/// return toward it, the reverse of the vector's numeric direction.
#[test]
fn arrow_navigation_follows_the_screen() {
    let commands = many(5);
    let mut state = State::new(&commands, "/here", Fuzzy::Smart);
    state.fit(10);

    state.up();
    assert_eq!(state.selected, 1);
    state.down();
    assert_eq!(state.selected, 0);
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
    // 12 rows: five rows of input chrome leave seven for the list.
    state.fit(12);
    assert_eq!(state.window, 7);

    state.move_by(6);
    assert_eq!(state.selected, 6);
    assert_eq!(state.offset, 0, "still in the first window");

    state.move_by(1);
    assert_eq!(state.selected, 7);
    assert_eq!(state.offset, 1, "scrolled by one");

    state.move_by(-7);
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
    assert_eq!(state.offset, 5, "the last window, not past it");
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
    assert_eq!(state.window, 2);
    assert!(
        state.selected >= state.offset && state.selected < state.offset + state.window,
        "selected {} is outside the window at {}..{}",
        state.selected,
        state.offset,
        state.offset + state.window
    );
}

/// Tab switches between all history and commands run in this exact directory. A parent directory
/// is useful as a ranking hint in global mode, but local means this directory only.
#[test]
fn tab_toggles_exact_directory_history() {
    let mut here = command("here", 3);
    here.dir = "/work/project/crate".to_string();
    let mut parent = command("parent", 2);
    parent.dir = "/work/project".to_string();
    let mut elsewhere = command("elsewhere", 1);
    elsewhere.dir = "/other".to_string();
    let commands = [here, parent, elsewhere];
    let mut state = State::new(&commands, "/work/project/crate", Fuzzy::Smart);

    assert_eq!(state.scope, Scope::Global);
    assert_eq!(state.matches.len(), 3);
    assert_eq!(state.total(), 3);

    state.toggle_scope();
    assert_eq!(state.scope, Scope::Local);
    assert_eq!(state.total(), 1);
    assert_eq!(state.matches.len(), 1);
    assert_eq!(state.matches[0].command.line, "here");

    state.toggle_scope();
    assert_eq!(state.scope, Scope::Global);
    assert_eq!(state.matches.len(), 3);
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

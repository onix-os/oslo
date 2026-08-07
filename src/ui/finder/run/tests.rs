//! The finder's own state: where the selection is, and where the window follows it to.
//!
//! What a keypress *means* is no longer decided here — the reader moved to
//! [`crate::ui::term`] when the finder stopped being the only thing that needed one, and
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
        session: String::new(),
        host: String::new(),
        root: None,
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
    let mut state = State::new(&commands, "/here", Fuzzy::Smart, "");
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
    let mut state = State::new(&commands, "/here", Fuzzy::Smart, "");
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
    let mut state = State::new(&commands, "/here", Fuzzy::Smart, "");
    // 12 rows: six rows of chrome (a margin at each edge, the gap, and the three-row surface)
    // leave six for the list.
    state.fit(12);
    assert_eq!(state.window, 6);

    state.move_by(5);
    assert_eq!(state.selected, 5);
    assert_eq!(state.offset, 0, "still in the first window");

    state.move_by(1);
    assert_eq!(state.selected, 6);
    assert_eq!(state.offset, 1, "scrolled by one");

    state.move_by(-6);
    assert_eq!(state.selected, 0);
    assert_eq!(state.offset, 0, "scrolled back");
}

/// The window never runs past the end of the list, which would draw blank rows between the last
/// match and the search bar and read as the list having ended early.
#[test]
fn the_window_does_not_overrun_the_list() {
    let commands = many(12);
    let mut state = State::new(&commands, "/here", Fuzzy::Smart, "");
    state.fit(12);
    state.move_by(100);
    assert_eq!(state.selected, 11);
    assert_eq!(state.offset, 6, "the last window, not past it");
}

/// Typing resets the selection: the old index referred to a list that no longer exists, and
/// keeping it would leave the cursor on an unrelated command.
#[test]
fn filtering_returns_to_the_top() {
    let commands = many(50);
    let mut state = State::new(&commands, "/here", Fuzzy::Smart, "");
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
    let mut state = State::new(&commands, "/here", Fuzzy::Smart, "");
    state.fit(40);
    state.move_by(30);
    assert_eq!(state.selected, 30);

    state.fit(7);
    assert_eq!(state.window, 1);
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
fn the_arrows_walk_the_scopes() {
    let mut here = command("here", 3);
    here.dir = "/work/project/crate".to_string();
    let mut parent = command("parent", 2);
    parent.dir = "/work/project".to_string();
    let mut elsewhere = command("elsewhere", 1);
    elsewhere.dir = "/other".to_string();
    let commands = [here, parent, elsewhere];
    let mut state = State::new(&commands, "/work/project/crate", Fuzzy::Smart, "");

    assert_eq!(state.scope, Scope::Global);
    assert_eq!(state.matches.len(), 3);
    assert_eq!(state.total(), 3);

    // Right walks widest to narrowest: global, host, session, directory, workspace, and round.
    state.narrow_scope();
    assert_eq!(state.scope, Scope::Host);
    state.narrow_scope();
    assert_eq!(state.scope, Scope::Session);
    state.narrow_scope();
    assert_eq!(state.scope, Scope::Directory);
    assert_eq!(state.total(), 1);
    assert_eq!(state.matches.len(), 1);
    assert_eq!(state.matches[0].command.line, "here");

    state.narrow_scope();
    assert_eq!(state.scope, Scope::Workspace);
    state.narrow_scope();
    assert_eq!(state.scope, Scope::Global);
    assert_eq!(state.matches.len(), 3);
}

/// Left walks back the way Right came, so the two are inverses at every step.
#[test]
fn the_arrows_are_inverses() {
    let commands = [command("anything", 100)];
    let mut state = State::new(&commands, "/here", Fuzzy::Smart, "");
    for _ in 0..5 {
        let was = state.scope;
        state.narrow_scope();
        assert_ne!(state.scope, was, "Right did not move");
        state.widen_scope();
        assert_eq!(state.scope, was, "Left did not come back");
        state.narrow_scope();
    }
    // And a full turn in either direction lands where it started.
    let start = state.scope;
    for _ in 0..5 {
        state.narrow_scope();
    }
    assert_eq!(state.scope, start, "Right does not wrap");
    for _ in 0..5 {
        state.widen_scope();
    }
    assert_eq!(state.scope, start, "Left does not wrap");
}

/// Each scope filters on what the store recorded, not on anything re-derived here.
#[test]
fn every_scope_filters_on_a_stored_fact() {
    let mut mine = command("mine", 100);
    mine.session = crate::track::session::id();
    mine.host = crate::track::session::host();
    mine.dir = "/work/app".to_string();
    mine.root = Some("/work/app".to_string());

    let mut theirs = command("theirs", 90);
    theirs.session = "some-other-shell".to_string();
    theirs.host = "another-machine".to_string();
    theirs.dir = "/elsewhere".to_string();
    theirs.root = None;

    let commands = [mine, theirs];
    let mut state = State::new(&commands, "/work/app", Fuzzy::Smart, "");
    // The shell's own worktree, which `State::new` would otherwise ask git for — and these paths
    // are not real directories.
    state.worktree = Some("/work/app".to_string());

    let lines = |state: &State| -> Vec<String> {
        state
            .matches
            .iter()
            .map(|row| row.command.line.clone())
            .collect()
    };

    assert_eq!(lines(&state), ["mine", "theirs"], "global shows both");
    state.narrow_scope(); // host
    assert_eq!(
        lines(&state),
        ["mine"],
        "another machine's row is filtered out"
    );
    state.narrow_scope(); // session
    assert_eq!(
        lines(&state),
        ["mine"],
        "another shell's row is filtered out"
    );
    state.narrow_scope(); // directory
    assert_eq!(lines(&state), ["mine"]);
    state.narrow_scope(); // workspace
    assert_eq!(lines(&state), ["mine"], "same worktree only");
}

/// A row written before sessions and hosts were recorded has neither. It still belongs to *this*
/// machine — there was only ever one — so `host` keeps showing it rather than hiding a history
/// somebody built up before the field existed.
#[test]
fn rows_from_before_the_fields_existed_still_show() {
    let mut old = command("from an older oslo", 100);
    old.session = String::new();
    old.host = String::new();
    let commands = [old];
    let mut state = State::new(&commands, "/here", Fuzzy::Smart, "");
    state.narrow_scope(); // host
    assert_eq!(state.matches.len(), 1, "an old row vanished from host");
    state.narrow_scope(); // session
    assert!(
        state.matches.is_empty(),
        "but it cannot claim to be from this session"
    );
}

/// A query that matches nothing leaves an empty list, and moving in it must not panic.
#[test]
fn an_empty_result_list_is_safe_to_move_in() {
    let commands = many(5);
    let mut state = State::new(&commands, "/here", Fuzzy::Smart, "");
    state.query.push_str("zzzzzzzz-no-such-command");
    state.refilter();
    assert!(state.matches.is_empty());
    state.fit(20);
    state.move_by(1);
    state.move_by(-1);
    assert_eq!(state.selected, 0);
}

/// **The typed line seeds the search.** Pressing Up after typing `ls` means "find the `ls` I ran
/// before"; opening on an empty query would throw the word away and ask for it again.
#[test]
fn the_seed_filters_the_list_immediately() {
    let commands = vec![
        command("ls -la /tmp", 100),
        command("echo alpha", 200),
        command("ls /etc", 50),
    ];
    let state = State::new(&commands, "/here", Fuzzy::Smart, "ls");
    assert_eq!(state.query, "ls");
    let lines: Vec<&str> = state
        .matches
        .iter()
        .map(|row| row.command.line.as_str())
        .collect();
    assert!(lines.contains(&"ls -la /tmp"), "{lines:?}");
    assert!(lines.contains(&"ls /etc"), "{lines:?}");
    assert!(!lines.contains(&"echo alpha"), "not filtered: {lines:?}");
}

/// A seed of only whitespace is no seed: opening the finder from a blank line must show
/// everything, not filter on a space.
#[test]
fn a_blank_seed_shows_everything() {
    let commands = vec![command("ls -la /tmp", 100), command("echo alpha", 200)];
    let state = State::new(&commands, "/here", Fuzzy::Smart, "   ");
    assert_eq!(state.query, "");
    assert_eq!(state.matches.len(), 2);
}

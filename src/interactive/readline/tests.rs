//! Readline key syntax, checked against the specs real integrations ship.
//!
//! The literal strings here were taken from `atuin init bash` and `hexe shp init bash` rather than
//! invented, because the syntax is only worth reading to the extent those parse.

use super::*;

fn one(spec: &str) -> KeyEvent {
    let events = parse_sequence(spec).expect("parses");
    assert_eq!(events.len(), 1, "{spec} is one key");
    events[0]
}

/// The two specs that started this: atuin's Ctrl-R and hexe's Ctrl-D.
#[test]
fn the_specs_shipped_by_real_integrations_parse() {
    assert_eq!(one(r#""\C-r""#), KeyEvent::ctrl('r'));
    assert_eq!(one(r#""\C-d""#), KeyEvent::ctrl('d'));
    // Written without the quotes just as often.
    assert_eq!(one(r"\C-r"), KeyEvent::ctrl('r'));
}

/// An arrow key arrives as the sequence the terminal sends, and has to be read as the key.
#[test]
fn escape_sequences_are_the_keys_they_stand_for() {
    assert_eq!(one(r#""\e[A""#), KeyEvent(KeyCode::Up, Modifiers::NONE));
    assert_eq!(one(r#""\e[B""#), KeyEvent(KeyCode::Down, Modifiers::NONE));
    assert_eq!(one(r#""\e[C""#), KeyEvent(KeyCode::Right, Modifiers::NONE));
    assert_eq!(one(r#""\e[D""#), KeyEvent(KeyCode::Left, Modifiers::NONE));
    assert_eq!(
        one(r#""\e[3~""#),
        KeyEvent(KeyCode::Delete, Modifiers::NONE)
    );
    assert_eq!(one(r#""\eOA""#), KeyEvent(KeyCode::Up, Modifiers::NONE));
}

#[test]
fn meta_is_alt() {
    assert_eq!(one(r#""\M-a""#), KeyEvent::alt('a'));
    // `\ea` with no bracket is Alt too, which is how readline writes it.
    assert_eq!(one(r#""\ea""#), KeyEvent::alt('a'));
}

/// Ctrl-I *is* Tab as far as a terminal is concerned. Binding it as `ctrl-i` would never fire,
/// so the ambiguous three are resolved to the key the terminal actually delivers.
#[test]
fn the_keys_a_terminal_cannot_tell_apart_resolve_to_what_it_sends() {
    assert_eq!(one(r#""\C-i""#), KeyEvent(KeyCode::Tab, Modifiers::NONE));
    assert_eq!(one(r#""\C-m""#), KeyEvent(KeyCode::Enter, Modifiers::NONE));
    assert_eq!(one(r#""\C-[""#), KeyEvent(KeyCode::Esc, Modifiers::NONE));
    assert_eq!(
        one(r#""\C-?""#),
        KeyEvent(KeyCode::Backspace, Modifiers::NONE)
    );
    assert_eq!(one(r#""\t""#), KeyEvent(KeyCode::Tab, Modifiers::NONE));
}

#[test]
fn a_sequence_is_more_than_one_key() {
    let events = parse_sequence(r#""\C-x\C-r""#).expect("parses");
    assert_eq!(events, vec![KeyEvent::ctrl('x'), KeyEvent::ctrl('r')]);
    let events = parse_sequence(r#""ab""#).expect("parses");
    assert_eq!(
        events,
        vec![
            KeyEvent(KeyCode::Char('a'), Modifiers::NONE),
            KeyEvent(KeyCode::Char('b'), Modifiers::NONE),
        ]
    );
}

#[test]
fn an_empty_spec_is_not_a_key() {
    assert_eq!(parse_sequence(""), None);
    assert_eq!(parse_sequence(r#""""#), None);
}

/// Rebinding a key replaces the old binding rather than stacking a second one, and `bind -r`
/// takes it off. Both go through the same key parse, so `"\C-r"` and `\C-r` name the same key.
#[test]
fn binding_is_by_key_not_by_spelling() {
    clear();
    bind(r#""\C-t""#, Bound::Command("first".into())).expect("binds");
    bind(r"\C-t", Bound::Command("second".into())).expect("rebinds");
    let entries = entries();
    let ours: Vec<_> = entries
        .iter()
        .filter(|e| e.keys == vec![KeyEvent::ctrl('t')])
        .collect();
    assert_eq!(ours.len(), 1, "one binding for one key");
    assert_eq!(ours[0].bound, Bound::Command("second".into()));

    assert!(unbind(r#""\C-t""#));
    assert!(!unbind(r#""\C-t""#), "already gone");
    clear();
}

/// The generation counter is what tells the read loop to re-apply, so every change must move it.
#[test]
fn every_change_moves_the_generation() {
    clear();
    let start = generation();
    bind(r#""\C-t""#, Bound::Command("x".into())).expect("binds");
    assert!(generation() > start);
    let after_bind = generation();
    unbind(r#""\C-t""#);
    assert!(generation() > after_bind);
    clear();
}

/// The request survives a round trip intact — this is the whole channel between the editor and
/// the read loop, and a dropped cursor position is a plugin that pastes in the wrong place.
#[test]
fn a_request_round_trips() {
    assert_eq!(take_request(), None);
    request("__atuin_history", "git comm", 8);
    let taken = take_request().expect("a request");
    assert_eq!(taken.command, "__atuin_history");
    assert_eq!(taken.line, "git comm");
    assert_eq!(taken.point, 8);
    assert_eq!(take_request(), None, "taken exactly once");
}
